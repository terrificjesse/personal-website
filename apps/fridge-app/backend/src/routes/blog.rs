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
    MAX_BLOG_TITLE_LENGTH, UpdateBlogPostRequest, exceeds_char_limit, is_blank, slugify,
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
    let offset = params.offset.unwrap_or(0);

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

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    // The count reuses the *same* WHERE, draft filter included. Counting without it would tell
    // a signed-out visitor exactly how many unpublished posts exist — the number leaks what
    // the rows themselves are hidden to protect.
    let count_sql = format!("SELECT COUNT(*) FROM blog_posts{where_clause}");
    let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(pattern) = &search {
        count_query = count_query.bind(pattern).bind(pattern);
    }
    let total = count_query
        .fetch_one(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // `sql_direction` returns a `&'static str` from a two-variant enum: `ORDER BY` can't take
    // a bind parameter, so this is the only user-influenced part of the statement that is
    // interpolated, and the enum is what keeps it from being user-*supplied*.
    //
    // `id` breaks ties, and paging is why it has to. File posts take `created_at` from a
    // frontmatter *day*, so they are all midnight and ties are the norm rather than the
    // exception; without a total order two pages can repeat a post and skip another.
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM blog_posts{where_clause} \
         ORDER BY created_at {}, id ASC LIMIT ? OFFSET ?",
        params.sort.sql_direction()
    );

    let mut query = sqlx::query_as::<_, BlogPost>(&sql);
    // Bound twice, as two separate `?` placeholders, because sqlx binds positionally in call
    // order — a numbered `?1` reused across both sides would not line up with one `.bind`.
    if let Some(pattern) = &search {
        query = query.bind(pattern).bind(pattern);
    }

    let posts = query
        .bind(limit)
        .bind(offset)
        .fetch_all(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(BlogPostPage {
        posts,
        total,
        limit,
        offset,
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

/// Appends `-2`, `-3`, ... until the slug is free. Collisions will be rare on a
/// single-admin blog, but two posts sharing a title shouldn't 500.
async fn unique_slug(pool: &SqlitePool, base: &str) -> Result<String, StatusCode> {
    let mut candidate = base.to_string();
    let mut suffix = 2;

    loop {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM blog_posts WHERE slug = ?)")
                .bind(&candidate)
                .fetch_one(pool)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        if !exists {
            return Ok(candidate);
        }

        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
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
    let slug = unique_slug(&pool, &base_slug).await?;

    let id = Uuid::new_v4().to_string();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO blog_posts \
         (id, author_id, title, slug, body, published, created_at, updated_at, source) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&user.id)
    .bind(title)
    .bind(&slug)
    .bind(&req.body)
    .bind(req.published)
    .bind(now)
    .bind(now)
    .bind(BLOG_SOURCE_DB)
    .execute(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
