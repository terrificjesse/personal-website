use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::{
    BlogPost, CreateBlogPostRequest, MAX_BLOG_BODY_LENGTH, MAX_BLOG_TITLE_LENGTH,
    UpdateBlogPostRequest, slugify,
};
use crate::routes::auth::{MaybeUser, RequireAdmin};

const SELECT_COLUMNS: &str = "id, author_id, title, slug, body, published, created_at, updated_at";

/// Lists posts. Signed-out visitors and non-admins see only published posts; an admin sees
/// drafts too, so there's somewhere to find unfinished work.
pub async fn list_posts(
    State(pool): State<SqlitePool>,
    MaybeUser(user): MaybeUser,
) -> Result<Json<Vec<BlogPost>>, StatusCode> {
    let is_admin = user.as_ref().is_some_and(|u| u.is_admin);

    let sql = if is_admin {
        format!("SELECT {SELECT_COLUMNS} FROM blog_posts ORDER BY created_at DESC")
    } else {
        format!(
            "SELECT {SELECT_COLUMNS} FROM blog_posts WHERE published = 1 ORDER BY created_at DESC"
        )
    };

    let posts = sqlx::query_as::<_, BlogPost>(&sql)
        .fetch_all(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(posts))
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
    if title.is_empty() || title.len() > MAX_BLOG_TITLE_LENGTH {
        return Err(StatusCode::BAD_REQUEST);
    }
    if req.body.is_empty() || req.body.len() > MAX_BLOG_BODY_LENGTH {
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
        "INSERT INTO blog_posts (id, author_id, title, slug, body, published, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&user.id)
    .bind(title)
    .bind(&slug)
    .bind(&req.body)
    .bind(req.published)
    .bind(now)
    .bind(now)
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

    if let Some(title) = req.title {
        let title = title.trim().to_string();
        if title.is_empty() || title.len() > MAX_BLOG_TITLE_LENGTH {
            return Err(StatusCode::BAD_REQUEST);
        }
        post.title = title;
    }
    if let Some(body) = req.body {
        if body.is_empty() || body.len() > MAX_BLOG_BODY_LENGTH {
            return Err(StatusCode::BAD_REQUEST);
        }
        post.body = body;
    }
    if let Some(published) = req.published {
        post.published = published;
    }
    post.updated_at = Utc::now();

    sqlx::query("UPDATE blog_posts SET title = ?, body = ?, published = ?, updated_at = ? WHERE id = ?")
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
    let result = sqlx::query("DELETE FROM blog_posts WHERE id = ?")
        .bind(&id)
        .execute(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(StatusCode::NO_CONTENT)
}
