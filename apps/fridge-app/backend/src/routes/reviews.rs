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
use crate::themealdb::Catalog as RecipeCatalog;

const SELECT_COLUMNS: &str = "id, recipe_id, rating, cooked_at, notes, user_id, is_public, hidden";

/// Who is making the current request. Always `None` until Phase 5 introduces sessions —
/// there are no accounts yet, so there is nobody else to be.
///
/// This exists as a named seam rather than a `None` literal sprinkled through the handlers:
/// every read path already threads its result into the visibility queries and into
/// `rerank_recommendations`, so Phase 5 replaces the body here (with a real session-user
/// extractor) and the rest of the review plumbing keeps working unchanged.
pub(crate) fn current_viewer() -> Option<String> {
    None
}

pub async fn submit_review(
    State(pool): State<SqlitePool>,
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
    let user_id = current_viewer();

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

/// Reviews written by `viewer`, most recently cooked first. Pre-Phase-5 (`viewer == None`)
/// there are no accounts, so this is every non-hidden row — which is still exactly "the
/// local user's reviews". Backs `GET /reviews` (the review-history page).
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
) -> Result<Json<Vec<ReviewWithRecipe>>, StatusCode> {
    let viewer = current_viewer();
    let reviews = fetch_for_viewer(&pool, viewer.as_deref()).await?;

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
/// — use `GET /reviews` for your own history. Once Phase 5 lands, this is also where a
/// per-recipe aggregate (count + smoothed mean) would be surfaced; see `docs/PLAN.md`.
pub async fn list_recipe_reviews(
    State(pool): State<SqlitePool>,
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
