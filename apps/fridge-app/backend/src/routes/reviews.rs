use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::{AddReviewRequest, Review, MAX_NOTES_LENGTH, MAX_RATING, MIN_RATING};
use crate::routes::auth::CurrentUser;
use crate::themealdb::Catalog as RecipeCatalog;

const SELECT_COLUMNS: &str = "id, recipe_id, rating, cooked_at, notes, user_id, is_public, hidden";

// Phase 4 left a `current_viewer()` seam here that returned `None` unconditionally. Phase 5
// replaced it with the `CurrentUser` extractor in `routes/auth.rs`: handlers now receive the
// session's user directly and pass `user.viewer()` into the same `Option<&str>` parameters
// that were already threaded through every read path. Nothing below the handler layer
// changed — see `routes/auth.rs`'s module doc for why that parameter stays an `Option`.

pub async fn submit_review(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AddReviewRequest>,
) -> Result<(StatusCode, Json<Review>), StatusCode> {
    if req.recipe_id.trim().is_empty() || !(MIN_RATING..=MAX_RATING).contains(&req.rating) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if req.notes.as_ref().is_some_and(|n| n.len() > MAX_NOTES_LENGTH) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let id = Uuid::new_v4().to_string();
    let cooked_at = req.cooked_at.unwrap_or_else(Utc::now);
    // Always `Some` now: the extractor rejects unauthenticated requests before this runs, so
    // no review written from here on can be an unclaimed NULL row.
    let user_id = Some(user.id.clone());

    sqlx::query(
        "INSERT INTO reviews (id, recipe_id, rating, cooked_at, notes, user_id, is_public, hidden) \
         VALUES (?, ?, ?, ?, ?, ?, ?, 0)",
    )
    .bind(&id)
    .bind(&req.recipe_id)
    .bind(req.rating)
    .bind(cooked_at)
    .bind(&req.notes)
    .bind(&user_id)
    .bind(req.is_public)
    .execute(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let review = Review {
        id,
        recipe_id: req.recipe_id,
        rating: req.rating,
        cooked_at,
        notes: req.notes,
        user_id,
        is_public: req.is_public,
        hidden: false,
    };

    Ok((StatusCode::CREATED, Json(review)))
}

/// Reviews written by `viewer`, most recently cooked first. Backs `GET /reviews` (the
/// review-history page).
///
/// The `viewer == None` branch is the pre-auth semantics: no accounts, so every non-hidden
/// row is the local user's. No handler reaches it any more — `CurrentUser` guarantees a real
/// id — but it stays because it's the documented meaning of `None` throughout the review
/// plumbing (`Review::is_by`), and because a NULL `user_id` in that branch is exactly the
/// unclaimed pre-Phase-5 row that `claim_unowned_rows` fixes at first registration.
pub(crate) async fn fetch_for_viewer(
    pool: &SqlitePool,
    viewer: Option<&str>,
) -> Result<Vec<Review>, StatusCode> {
    let sql = match viewer {
        None => format!("SELECT {SELECT_COLUMNS} FROM reviews WHERE hidden = 0 ORDER BY cooked_at DESC"),
        Some(_) => format!(
            "SELECT {SELECT_COLUMNS} FROM reviews \
             WHERE hidden = 0 AND user_id = ? ORDER BY cooked_at DESC"
        ),
    };

    let mut query = sqlx::query_as::<_, Review>(&sql);
    if let Some(viewer_id) = viewer {
        query = query.bind(viewer_id);
    }

    query
        .fetch_all(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Every review `viewer` is allowed to see: their own (public or not) plus everyone else's
/// public ones. This is the input to `rerank::rerank_recommendations` — it deliberately
/// carries both populations in one slice rather than pre-splitting them, because deciding
/// how to weight your own feedback against the crowd's is that function's job. Use
/// `Review::is_by` to tell them apart.
pub(crate) async fn fetch_visible_to(
    pool: &SqlitePool,
    viewer: Option<&str>,
) -> Result<Vec<Review>, StatusCode> {
    let sql = match viewer {
        // Pre-Phase-5: no accounts, so every row is the local user's own.
        None => format!("SELECT {SELECT_COLUMNS} FROM reviews WHERE hidden = 0 ORDER BY cooked_at DESC"),
        Some(_) => format!(
            "SELECT {SELECT_COLUMNS} FROM reviews \
             WHERE hidden = 0 AND (user_id = ? OR is_public = 1) ORDER BY cooked_at DESC"
        ),
    };

    let mut query = sqlx::query_as::<_, Review>(&sql);
    if let Some(viewer_id) = viewer {
        query = query.bind(viewer_id);
    }

    query
        .fetch_all(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// One review plus enough recipe context for the review-history page to render without a
/// second fetch. `recipe_name`/`recipe_image_url` come from the vendored catalog looked up
/// by `recipe_id`, not from the `reviews` table.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewWithRecipe {
    #[serde(flatten)]
    pub review: Review,
    pub recipe_name: String,
    pub recipe_image_url: Option<String>,
}

fn with_recipe(catalog: &RecipeCatalog, review: Review) -> ReviewWithRecipe {
    let recipe = catalog.recipes().iter().find(|r| r.id == review.recipe_id);
    ReviewWithRecipe {
        recipe_name: recipe
            .map(|r| r.name.clone())
            .unwrap_or_else(|| "Unknown recipe".to_string()),
        recipe_image_url: recipe.and_then(|r| r.image_url.clone()),
        review,
    }
}

/// `GET /reviews` — the signed-in user's own review history.
pub async fn list_reviews(
    State(pool): State<SqlitePool>,
    State(catalog): State<Arc<RecipeCatalog>>,
    user: CurrentUser,
) -> Result<Json<Vec<ReviewWithRecipe>>, StatusCode> {
    let reviews = fetch_for_viewer(&pool, user.viewer()).await?;

    Ok(Json(
        reviews
            .into_iter()
            .map(|review| with_recipe(&catalog, review))
            .collect(),
    ))
}

/// `GET /recipes/{id}/reviews` — the public review wall for one recipe: everyone's opt-in
/// reviews, newest first. The read half of the global aggregator.
///
/// Returns only `is_public` rows, so it never leaks a private review even to its own author
/// — use `GET /reviews` for your own history. This is also where a per-recipe aggregate
/// (count + smoothed mean) would be surfaced; that's the deferred small-sample-statistics
/// `[learn]` item in `docs/PLAN.md`.
///
/// Requires a session even though every row it returns is public. Nothing here is secret, but
/// an unauthenticated endpoint on a backend bound to `0.0.0.0` is the exact thing PLAN.md
/// warned about, and this app has no anonymous-browsing story to justify the exception.
pub async fn list_recipe_reviews(
    State(pool): State<SqlitePool>,
    _user: CurrentUser,
    Path(recipe_id): Path<String>,
) -> Result<Json<Vec<Review>>, StatusCode> {
    let reviews = sqlx::query_as::<_, Review>(&format!(
        "SELECT {SELECT_COLUMNS} FROM reviews \
         WHERE recipe_id = ? AND is_public = 1 AND hidden = 0 ORDER BY cooked_at DESC"
    ))
    .bind(&recipe_id)
    .fetch_all(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(reviews))
}
