use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::{AddReviewRequest, MAX_NOTES_LENGTH, MAX_RATING, MIN_RATING, Review};
use crate::rate_limit::{ClientIp, RateLimits};
use crate::routes::auth::CurrentUser;
use crate::themealdb::Catalog as RecipeCatalog;

const SELECT_COLUMNS: &str = "id, recipe_id, rating, cooked_at, notes, user_id, is_public, hidden";

// Adds a review for a recipe from a user. Does basic sanity checking verifying
// that the recipe_id isn't empty, the rating is valid, and the note length is within
// the MAX_NOTES_LENGTH. Then proceeds to insert into the reviews database pool

pub async fn submit_review(
    State(pool): State<SqlitePool>,
    State(rate_limits): State<RateLimits>,
    ClientIp(ip): ClientIp,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AddReviewRequest>,
) -> axum::response::Response {
    if let Err(retry) = rate_limits.check_review(ip, &user.id) {
        return retry.response("POST /reviews", ip, &user.id);
    }

    submit_review_after_rate_limit(&pool, user.id, req)
        .await
        .into_response()
}

async fn submit_review_after_rate_limit(
    pool: &SqlitePool,
    user_id: String,
    req: AddReviewRequest,
) -> Result<(StatusCode, Json<Review>), StatusCode> {
    if req.recipe_id.trim().is_empty() || !(MIN_RATING..=MAX_RATING).contains(&req.rating) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if req
        .notes
        .as_ref()
        .is_some_and(|n| n.len() > MAX_NOTES_LENGTH)
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    let id = Uuid::new_v4().to_string();
    let cooked_at = req.cooked_at.unwrap_or_else(Utc::now);
    let user_id = Some(user_id);

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
    .execute(pool)
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

/// Grabs reviews based on the viewer, where a None viewer indicates all nonhidden reviews
/// and a viewer with viewer_id will filter out to only the reviews they wrote.
pub(crate) async fn fetch_for_viewer(
    pool: &SqlitePool,
    viewer: Option<&str>,
) -> Result<Vec<Review>, StatusCode> {
    let sql = match viewer {
        None => {
            format!("SELECT {SELECT_COLUMNS} FROM reviews WHERE hidden = 0 ORDER BY cooked_at DESC")
        }
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

/// Grabs reviews based on the viewer, where a None viewer indicates all reviews
/// and a viewer with viewer_id will filter out to the reviews associated with that viewer_id
/// and other publically viewable reviews.
pub(crate) async fn fetch_visible_to(
    pool: &SqlitePool,
    viewer: Option<&str>,
) -> Result<Vec<Review>, StatusCode> {
    let sql = match viewer {
        None => {
            format!("SELECT {SELECT_COLUMNS} FROM reviews WHERE hidden = 0 ORDER BY cooked_at DESC")
        }
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

/// Associates a review with its specific recipe:
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

/// Lists all of the reviews written by the user:
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

/// Lists all of the public reviews for a given recipe with matching recipe_id.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::User;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::net::IpAddr;

    #[tokio::test]
    async fn the_review_route_refuses_the_thirty_first_submission_without_inserting_it() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let rate_limits = RateLimits::default();
        let ip = IpAddr::from([192, 0, 2, 1]);
        let user = User {
            id: "user-1".to_string(),
            email: "reviewer@example.com".to_string(),
            password_hash: None,
            created_at: Utc::now(),
            is_admin: false,
        };

        for attempt in 0..=30 {
            let response = submit_review(
                State(pool.clone()),
                State(rate_limits.clone()),
                ClientIp(ip),
                CurrentUser(user.clone()),
                Json(AddReviewRequest {
                    recipe_id: format!("recipe-{attempt}"),
                    rating: 5,
                    notes: None,
                    cooked_at: None,
                    is_public: false,
                }),
            )
            .await;

            let expected = if attempt < 30 {
                StatusCode::CREATED
            } else {
                StatusCode::TOO_MANY_REQUESTS
            };
            assert_eq!(response.status(), expected, "submission {}", attempt + 1);
        }

        let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reviews")
            .fetch_one(&pool)
            .await
            .expect("review count");
        assert_eq!(stored, 30);
    }
}
