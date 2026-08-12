use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use sqlx::SqlitePool;

use crate::models::{Recipe, LIKED_RATING_THRESHOLD};
use crate::recommend_recipes::{self, RecipeFilters, RecommendedRecipe};
use crate::rerank;
use crate::routes::{items, reviews, shopping_list};
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

/// `GET /recipes/liked` — the "recipes you liked" section (Phase 4), separate from the
/// general recommendations above. Membership is a plain threshold (`LIKED_RATING_THRESHOLD`:
/// has the user rated this recipe highly at least once?); ordering among that set is
/// `rerank::rerank_recommendations`'s job. See its module doc for why the split is drawn
/// there.
pub async fn liked(
    State(pool): State<SqlitePool>,
    State(catalog): State<Arc<Catalog>>,
) -> Result<Json<Vec<Recipe>>, StatusCode> {
    let reviews = reviews::fetch_all(&pool).await?;
    let liked_recipe_ids: HashSet<&str> = reviews
        .iter()
        .filter(|review| review.rating >= LIKED_RATING_THRESHOLD)
        .map(|review| review.recipe_id.as_str())
        .collect();

    let candidates: Vec<Recipe> = catalog
        .recipes()
        .iter()
        .filter(|recipe| liked_recipe_ids.contains(recipe.id.as_str()))
        .cloned()
        .collect();

    Ok(Json(rerank::rerank_recommendations(&candidates, &reviews)))
}
