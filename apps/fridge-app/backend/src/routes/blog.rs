use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::blog_files::{self, SyncReport};
use crate::models::{
    BLOG_SOURCE_DB, BLOG_SOURCE_FILE, BlogPost, BlogPostPage, CreateBlogPostRequest,
    DEFAULT_BLOG_PAGE_SIZE, ListPostsQuery, MAX_BLOG_BODY_LENGTH, MAX_BLOG_PAGE_SIZE,
    MAX_BLOG_TITLE_LENGTH, SortOrder, UpdateBlogPostRequest, decode_cursor, encode_cursor,
    exceeds_char_limit, is_blank, slugify,
};
use crate::routes::auth::{MaybeUser, RequireAdmin};

const SELECT_COLUMNS: &str =
    "id, author_id, title, slug, body, published, created_at, updated_at, source";

/// Wraps a search term for use with SQL `LIKE`.
///
/// `LIKE` gives `%` and `_` their own meaning, so a term containing either would otherwise
/// match far more than the user typed — searching for `100%` would return every post, and
/// `a_b` would match `axb`. Escaping them (and the escape character itself, first, or the
/// escapes we add would be re-escaped) makes the search literal, which is what a reader
/// typing into a search box expects. The statement pairs this with `ESCAPE '\\'`.
///
/// Case-insensitivity comes free from SQLite's default `LIKE`, but only over ASCII — a
/// non-ASCII term matches case-sensitively. Fine for now; it's the same limitation FTS5
/// without an ICU tokenizer would have.
fn like_pattern(term: &str) -> String {
    let escaped = term
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");

    format!("%{escaped}%")
}

/// The 409 a file-sourced post answers write attempts with.
///
/// File posts are owned by `blog_files::sync`, not by the API: the next sync rewrites the row
/// from disk, so an accepted edit would silently disappear. Refusing is the honest answer, and
/// it belongs here rather than only in the admin UI — the UI hiding a button is optimistic,
/// exactly as `docs/BLOG.md` says of `proxy.ts` and the `is_admin` check.
///
/// 409 rather than 403: the request isn't unauthorized (the same admin may edit any db post),
/// it conflicts with the state of this particular resource.
fn reject_if_file_sourced(post: &BlogPost) -> Result<(), StatusCode> {
    if post.source == BLOG_SOURCE_FILE {
        return Err(StatusCode::CONFLICT);
    }
    Ok(())
}

/// Lists posts. Signed-out visitors and non-admins see only published posts; an admin sees
/// drafts too, so there's somewhere to find unfinished work.
pub async fn list_posts(
    State(pool): State<SqlitePool>,
    MaybeUser(user): MaybeUser,
    Query(params): Query<ListPostsQuery>,
) -> Result<Json<BlogPostPage>, StatusCode> {
    let is_admin = user.as_ref().is_some_and(|u| u.is_admin);

    let limit = params.limit.unwrap_or(DEFAULT_BLOG_PAGE_SIZE);
    // Refused rather than clamped. A caller who asks for 1000 and silently receives 100
    // believes it now holds every post — the same looks-complete-but-isn't failure that makes
    // an unrecognized `?sort=` a 400 instead of a quiet fallback.
    if limit == 0 || limit > MAX_BLOG_PAGE_SIZE {
        return Err(StatusCode::BAD_REQUEST);
    }

    // A cursor that cannot be read is a caller error, not an empty first page.
    let cursor = match params.cursor.as_deref() {
        None => None,
        Some(raw) => Some(decode_cursor(raw).ok_or(StatusCode::BAD_REQUEST)?),
    };

    // One statement over one table covers both post kinds. That is the entire reason
    // file-sourced posts are rows rather than a second store read at request time: search and
    // sort are written once here and apply to everything, instead of being reimplemented in
    // Rust over a merged list.
    let mut conditions: Vec<&str> = Vec::new();
    if !is_admin {
        conditions.push("published = 1");
    }

    // Whitespace-only is treated as no search at all, so a stray space in the search box
    // doesn't come back as zero results.
    let search = params
        .q
        .as_deref()
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(like_pattern);
    if search.is_some() {
        conditions.push("(title LIKE ? ESCAPE '\\' OR body LIKE ? ESCAPE '\\')");
    }

    // Everything above describes the whole result set; the cursor narrows it to one page. The
    // count below therefore uses only what is above, so "showing 20 of 143" keeps saying 143
    // as you advance rather than counting down what is left.
    let count_where = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let count_sql = format!("SELECT COUNT(*) FROM blog_posts{count_where}");
    let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
    // The count reuses the same draft filter and search. Counting without the draft filter
    // would tell a signed-out visitor exactly how many unpublished posts exist — the number
    // leaks what the rows themselves are hidden to protect.
    if let Some(pattern) = &search {
        count_query = count_query.bind(pattern).bind(pattern);
    }
    let total = count_query
        .fetch_one(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // The cursor condition has to mirror the ORDER BY exactly, and the ORDER BY is mixed —
    // `created_at` follows the sort direction while `id` is always ascending — so this cannot
    // be written as a single row-value comparison. Getting it wrong does not error; it
    // silently skips or repeats rows, which is the bug this replaced.
    if cursor.is_some() {
        conditions.push(match params.sort {
            SortOrder::Newest => "(created_at < ? OR (created_at = ? AND id > ?))",
            SortOrder::Oldest => "(created_at > ? OR (created_at = ? AND id > ?))",
        });
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    // `sql_direction` returns a `&'static str` from a two-variant enum: `ORDER BY` can't take
    // a bind parameter, so this is the only user-influenced part of the statement that is
    // interpolated, and the enum is what keeps it from being user-*supplied*.
    //
    // `id` breaks ties, and paging is why it has to. File posts take `created_at` from a
    // frontmatter *day*, so they are all midnight and ties are the norm rather than the
    // exception; without a total order there is no single "next row" for a cursor to name.
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM blog_posts{where_clause} \
         ORDER BY created_at {}, id ASC LIMIT ?",
        params.sort.sql_direction()
    );

    let mut query = sqlx::query_as::<_, BlogPost>(&sql);
    // Bound in the order the conditions were pushed, because sqlx binds positionally: search
    // first (twice, as two separate `?`), then the cursor's three.
    if let Some(pattern) = &search {
        query = query.bind(pattern).bind(pattern);
    }
    if let Some((created_at, id)) = &cursor {
        query = query.bind(created_at).bind(created_at).bind(id);
    }

    // One more than asked for: whether a further page exists is not answerable from a full
    // page, and `total` cannot answer it either once a cursor is in play.
    let mut posts = query
        .bind(limit + 1)
        .fetch_all(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let has_more = posts.len() > limit as usize;
    posts.truncate(limit as usize);
    let next_cursor = has_more
        .then(|| posts.last())
        .flatten()
        .map(|post| encode_cursor(post.created_at, &post.id));

    Ok(Json(BlogPostPage {
        posts,
        total,
        limit,
        next_cursor,
    }))
}

/// A single post by its slug. A draft 404s for a non-admin rather than answering with
/// something like 403 — that would confirm an unpublished post exists at that slug.
pub async fn get_post(
    State(pool): State<SqlitePool>,
    MaybeUser(user): MaybeUser,
    Path(slug): Path<String>,
) -> Result<Json<BlogPost>, StatusCode> {
    let is_admin = user.as_ref().is_some_and(|u| u.is_admin);

    let post = sqlx::query_as::<_, BlogPost>(&format!(
        "SELECT {SELECT_COLUMNS} FROM blog_posts WHERE slug = ?"
    ))
    .bind(&slug)
    .fetch_optional(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;

    if !post.published && !is_admin {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(post))
}

/// The most `-2`, `-3`, … suffixes to try before giving up.
///
/// A bound rather than an open loop: every retry here is driven by a database error, and if
/// one ever arrives for a reason other than the slug — a corrupt index, say — an unbounded
/// loop would spin against the database forever instead of failing.
const MAX_SLUG_ATTEMPTS: u32 = 50;

/// Inserts a post, letting the database pick the slug suffix.
///
/// This deliberately does **not** ask whether a slug is free first. It used to: a
/// `SELECT EXISTS(...)` followed by an `INSERT`, with nothing holding between them. Two
/// requests could both see the same slug free, and the loser hit the `UNIQUE` constraint and
/// came back **500**. That is not a theoretical race — ten concurrent creates of the same
/// title produced six 500s, and one double-clicked submit button is enough to reach it.
///
/// The constraint is the only thing that can decide atomically, so it decides. We attempt the
/// insert and treat a unique violation as "someone took that one", trying the next suffix.
/// Anything else is a real error.
/// Everything a new post needs, bundled so the insert takes two arguments rather than eight.
struct NewPost<'a> {
    id: &'a str,
    author_id: &'a str,
    title: &'a str,
    base_slug: &'a str,
    body: &'a str,
    published: bool,
    now: chrono::DateTime<Utc>,
}

async fn insert_post_with_unique_slug(
    pool: &SqlitePool,
    post: &NewPost<'_>,
) -> Result<String, StatusCode> {
    for attempt in 1..=MAX_SLUG_ATTEMPTS {
        let candidate = if attempt == 1 {
            post.base_slug.to_string()
        } else {
            format!("{}-{attempt}", post.base_slug)
        };

        let result = sqlx::query(
            "INSERT INTO blog_posts \
             (id, author_id, title, slug, body, published, created_at, updated_at, source) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(post.id)
        .bind(post.author_id)
        .bind(post.title)
        .bind(&candidate)
        .bind(post.body)
        .bind(post.published)
        .bind(post.now)
        .bind(post.now)
        .bind(BLOG_SOURCE_DB)
        .execute(pool)
        .await;

        match result {
            Ok(_) => return Ok(candidate),
            // Only a *slug* collision is retryable. `is_unique_violation` would also fire on
            // the `id` primary key, but `id` is a fresh v4 UUID and is not re-rolled here, so a
            // genuine id collision exhausts the attempts and 500s rather than looping.
            Err(sqlx::Error::Database(err)) if err.is_unique_violation() => continue,
            Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    Err(StatusCode::INTERNAL_SERVER_ERROR)
}

/// Creates a post. Admin-only.
pub async fn create_post(
    State(pool): State<SqlitePool>,
    RequireAdmin(user): RequireAdmin,
    Json(req): Json<CreateBlogPostRequest>,
) -> Result<(StatusCode, Json<BlogPost>), StatusCode> {
    let title = req.title.trim();
    if is_blank(title) || exceeds_char_limit(title, MAX_BLOG_TITLE_LENGTH) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if is_blank(&req.body) || exceeds_char_limit(&req.body, MAX_BLOG_BODY_LENGTH) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let base_slug = slugify(title);
    if base_slug.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now();

    let slug = insert_post_with_unique_slug(
        &pool,
        &NewPost {
            id: &id,
            author_id: &user.id,
            title,
            base_slug: &base_slug,
            body: &req.body,
            published: req.published,
            now,
        },
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(BlogPost {
            id,
            author_id: user.id,
            title: title.to_string(),
            slug,
            body: req.body,
            published: req.published,
            created_at: now,
            updated_at: now,
            source: BLOG_SOURCE_DB.to_string(),
        }),
    ))
}

/// Updates a post. Admin-only, and partial — only fields present in the request change.
/// Deliberately never rewrites `slug`, even when `title` changes, so a published URL stays
/// stable once it's out in the world.
pub async fn update_post(
    State(pool): State<SqlitePool>,
    RequireAdmin(_user): RequireAdmin,
    Path(id): Path<String>,
    Json(req): Json<UpdateBlogPostRequest>,
) -> Result<Json<BlogPost>, StatusCode> {
    let mut post = sqlx::query_as::<_, BlogPost>(&format!(
        "SELECT {SELECT_COLUMNS} FROM blog_posts WHERE id = ?"
    ))
    .bind(&id)
    .fetch_optional(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;

    reject_if_file_sourced(&post)?;

    if let Some(title) = req.title {
        let title = title.trim().to_string();
        if is_blank(&title) || exceeds_char_limit(&title, MAX_BLOG_TITLE_LENGTH) {
            return Err(StatusCode::BAD_REQUEST);
        }
        post.title = title;
    }
    if let Some(body) = req.body {
        if is_blank(&body) || exceeds_char_limit(&body, MAX_BLOG_BODY_LENGTH) {
            return Err(StatusCode::BAD_REQUEST);
        }
        post.body = body;
    }
    if let Some(published) = req.published {
        post.published = published;
    }
    post.updated_at = Utc::now();

    sqlx::query(
        "UPDATE blog_posts SET title = ?, body = ?, published = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&post.title)
    .bind(&post.body)
    .bind(post.published)
    .bind(post.updated_at)
    .bind(&post.id)
    .execute(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(post))
}

/// Deletes a post. Admin-only.
pub async fn delete_post(
    State(pool): State<SqlitePool>,
    RequireAdmin(_user): RequireAdmin,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    // Reads the row before deleting rather than checking `rows_affected`, because a
    // file-sourced post has to be told apart from a missing one: 409 vs 404. Deleting one
    // would only bring it back at the next sync anyway — remove the file instead.
    let post = sqlx::query_as::<_, BlogPost>(&format!(
        "SELECT {SELECT_COLUMNS} FROM blog_posts WHERE id = ?"
    ))
    .bind(&id)
    .fetch_optional(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;

    reject_if_file_sourced(&post)?;

    sqlx::query("DELETE FROM blog_posts WHERE id = ?")
        .bind(&id)
        .execute(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Re-reads `content/blog/` and reconciles it with the database. Admin-only.
///
/// The same sync runs at startup; this endpoint exists so that publishing a pushed `.md` file
/// doesn't require restarting the backend. Returns what changed, so "nothing happened" is
/// distinguishable from "four posts were skipped" without reading the server log.
pub async fn sync_posts(
    State(pool): State<SqlitePool>,
    RequireAdmin(_user): RequireAdmin,
) -> Result<Json<SyncReport>, StatusCode> {
    // `RequireAdmin` guarantees an admin exists, so the no-admin deferral is unreachable from
    // this route; `report()` flattens any other deferral to zeros, which is what it did before.
    let report = blog_files::sync(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .report();

    Ok(Json(report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_term_is_wrapped_in_wildcards() {
        assert_eq!(like_pattern("rust"), "%rust%");
    }

    /// The bug this prevents: without escaping, `%` is the LIKE wildcard, so searching for it
    /// matches every post rather than the ones mentioning a percentage.
    #[test]
    fn sql_wildcards_in_the_term_are_escaped() {
        assert_eq!(like_pattern("100%"), r"%100\%%");
        assert_eq!(like_pattern("a_b"), r"%a\_b%");
    }

    /// The escape character has to be escaped first, or the backslashes added for `%` and `_`
    /// would themselves be doubled and the pattern would stop meaning what it says.
    #[test]
    fn the_escape_character_is_escaped_before_the_wildcards() {
        assert_eq!(like_pattern(r"\"), r"%\\%");
        assert_eq!(like_pattern(r"a\%b"), r"%a\\\%b%");
    }

    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool_with_author() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory database should open");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations should apply");
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, created_at, is_admin) \
             VALUES ('a1', 'a@example.com', NULL, ?, 1)",
        )
        .bind(Utc::now())
        .execute(&pool)
        .await
        .expect("author should insert");
        pool
    }

    async fn insert(pool: &SqlitePool, base: &str) -> Result<String, StatusCode> {
        insert_post_with_unique_slug(
            pool,
            &NewPost {
                id: &Uuid::new_v4().to_string(),
                author_id: "a1",
                title: "Race Me",
                base_slug: base,
                body: "Body",
                published: true,
                now: Utc::now(),
            },
        )
        .await
    }

    /// **J1.** Posts sharing a title must each get their own slug, and none may fail.
    ///
    /// The old implementation asked `SELECT EXISTS(...)` and then inserted. Nothing held
    /// between the two, so under concurrency the loser hit the UNIQUE constraint and returned
    /// 500 — six times out of ten in a ten-way race. Letting the constraint arbitrate is what
    /// makes the suffix assignment atomic.
    #[tokio::test]
    async fn posts_sharing_a_title_each_get_their_own_slug() {
        let pool = pool_with_author().await;

        let mut slugs = Vec::new();
        for _ in 0..5 {
            slugs.push(
                insert(&pool, "race-me")
                    .await
                    .expect("no create may fail because another took the slug"),
            );
        }

        assert_eq!(
            slugs,
            vec![
                "race-me",
                "race-me-2",
                "race-me-3",
                "race-me-4",
                "race-me-5"
            ],
            "the suffix sequence is unchanged from the check-then-insert version"
        );
    }

    /// A slug already taken by a *file*-sourced post is skipped over just the same — the
    /// constraint does not care which kind of post holds it.
    #[tokio::test]
    async fn a_slug_held_by_a_file_post_is_stepped_over() {
        let pool = pool_with_author().await;
        sqlx::query(
            "INSERT INTO blog_posts \
             (id, author_id, title, slug, body, published, created_at, updated_at, source) \
             VALUES ('f1', 'a1', 'From File', 'race-me', 'B', 1, ?, ?, 'file')",
        )
        .bind(Utc::now())
        .bind(Utc::now())
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(insert(&pool, "race-me").await.unwrap(), "race-me-2");
    }

    #[test]
    fn only_file_sourced_posts_are_refused() {
        let mut post = BlogPost {
            id: "id".to_string(),
            author_id: "author".to_string(),
            title: "T".to_string(),
            slug: "t".to_string(),
            body: "B".to_string(),
            published: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            source: BLOG_SOURCE_DB.to_string(),
        };
        assert!(reject_if_file_sourced(&post).is_ok());

        post.source = BLOG_SOURCE_FILE.to_string();
        assert_eq!(
            reject_if_file_sourced(&post),
            Err(StatusCode::CONFLICT),
            "a file-sourced post must be refused, and with 409 rather than 403 or 404"
        );
    }
}
