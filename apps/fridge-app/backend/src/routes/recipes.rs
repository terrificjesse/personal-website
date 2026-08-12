use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use sqlx::SqlitePool;

use crate::recommend_recipes::{self, RecipeFilters, RecommendedRecipe};
use crate::routes::{items, shopping_list};
use crate::themealdb::Catalog;

/// `GET /recipes/recommended?cuisine=&mealType=` — the full vendored catalog, filtered and
/// ranked against current fridge + shopping-list contents. See `recommend_recipes`'s module
/// doc for what the filters do and how ranking is meant to work.
pub async fn recommended(
    State(pool): State<SqlitePool>,
    State(catalog): State<Arc<Catalog>>,
    Query(filters): Query<RecipeFilters>,
) -> Result<Json<Vec<RecommendedRecipe>>, StatusCode> {
    let fridge = items::fetch_all(&pool).await?;
    let shopping_list = shopping_list::fetch_all(&pool).await?;

    Ok(Json(recommend_recipes::recommend_recipes(
        catalog.recipes(),
        &fridge,
        &shopping_list,
        &filters,
    )))
}
