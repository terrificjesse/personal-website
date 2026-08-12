use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::{AddReviewRequest, Review, MAX_RATING, MIN_RATING};
use crate::themealdb::Catalog as RecipeCatalog;

pub async fn submit_review(
    State(pool): State<SqlitePool>,
    Json(req): Json<AddReviewRequest>,
) -> Result<(StatusCode, Json<Review>), StatusCode> {
    if req.recipe_id.trim().is_empty() || !(MIN_RATING..=MAX_RATING).contains(&req.rating) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let id = Uuid::new_v4().to_string();
    let cooked_at = req.cooked_at.unwrap_or_else(Utc::now);

    sqlx::query(
        "INSERT INTO reviews (id, recipe_id, rating, cooked_at, notes) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&req.recipe_id)
    .bind(req.rating)
    .bind(cooked_at)
    .bind(&req.notes)
    .execute(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let review = Review {
        id,
        recipe_id: req.recipe_id,
        rating: req.rating,
        cooked_at,
        notes: req.notes,
    };

    Ok((StatusCode::CREATED, Json(review)))
}

/// All reviews, most recently cooked first. Shared by `GET /reviews` and `GET
/// /recipes/liked`, which needs full review history as input to
/// `rerank::rerank_recommendations` — same reasoning as `items::fetch_all`.
pub(crate) async fn fetch_all(pool: &SqlitePool) -> Result<Vec<Review>, StatusCode> {
    sqlx::query_as::<_, Review>(
        "SELECT id, recipe_id, rating, cooked_at, notes FROM reviews ORDER BY cooked_at DESC",
    )
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

pub async fn list_reviews(
    State(pool): State<SqlitePool>,
    State(catalog): State<Arc<RecipeCatalog>>,
) -> Result<Json<Vec<ReviewWithRecipe>>, StatusCode> {
    let reviews = fetch_all(&pool).await?;

    let result = reviews
        .into_iter()
        .map(|review| {
            let recipe = catalog.recipes().iter().find(|r| r.id == review.recipe_id);
            ReviewWithRecipe {
                recipe_name: recipe
                    .map(|r| r.name.clone())
                    .unwrap_or_else(|| "Unknown recipe".to_string()),
                recipe_image_url: recipe.and_then(|r| r.image_url.clone()),
                review,
            }
        })
        .collect();

    Ok(Json(result))
}
