use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::foodkeeper::Catalog;
use crate::nlp::{Candidate, Suggestion, SuggestionSource, suggest_item_names};
use crate::routes::auth::CurrentUser;

const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 25;

#[derive(Debug, Deserialize)]
pub struct SuggestQuery {
    #[serde(default)]
    pub q: String,
    pub limit: Option<usize>,
}

// Suggests params.limit many items based on the given params.query:
pub async fn suggest_items(
    State(pool): State<SqlitePool>,
    State(catalog): State<Arc<Catalog>>,
    CurrentUser(user): CurrentUser,
    Query(params): Query<SuggestQuery>,
) -> Result<Json<Vec<Suggestion>>, StatusCode> {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let query = params.q.trim();

    let fridge_names = fetch_fridge_names(&pool, &user.id).await?;

    if query.is_empty() {
        // Take the top limit many rows of the fridge as default for an empty query:
        return Ok(Json(
            fridge_names
                .into_iter()
                .take(limit)
                .map(|(name, foodkeeper_product_id)| Suggestion {
                    name,
                    source: SuggestionSource::Fridge,
                    foodkeeper_product_id,
                    score: 1.0,
                })
                .collect(),
        ));
    }

    // Converts the entries for products already in the fridge into a hashmap for ease of access
    let mut candidates: Vec<Candidate> = fridge_names
        .into_iter()
        .map(|(name, foodkeeper_product_id)| Candidate {
            name_lower: name.to_lowercase(),
            name,
            source: SuggestionSource::Fridge,
            aliases: Vec::new(),
            foodkeeper_product_id,
        })
        .collect();

    let fridge_by_name: HashMap<String, usize> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| (candidate.name_lower.clone(), index))
        .collect();

    // Coalesce the fridge entries and the FoodKeeper entries
    for entry in catalog.entries() {
        match fridge_by_name.get(&entry.name_lower) {
            Some(&index) => {
                let candidate = &mut candidates[index];
                candidate.aliases.extend(entry.aliases.iter().cloned());
                if candidate.foodkeeper_product_id.is_none() {
                    candidate.foodkeeper_product_id = entry.product_ids.first().copied();
                }
            }
            None => candidates.push(Candidate {
                name: entry.name.clone(),
                name_lower: entry.name_lower.clone(),
                source: SuggestionSource::Foodkeeper,
                aliases: entry.aliases.clone(),
                foodkeeper_product_id: entry.product_ids.first().copied(),
            }),
        }
    }

    // Hand off to the ranking by score function to actually rank the items
    Ok(Json(suggest_item_names(query, &candidates, limit)))
}

/// Fetches all of the names and product_ids from entries grouped by canonical names within the fridge:
async fn fetch_fridge_names(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<(String, Option<i64>)>, StatusCode> {
    let rows = sqlx::query_as::<_, (String, Option<i64>, String)>(
        "SELECT canonical_name, foodkeeper_product_id, MAX(added_at) AS latest \
         FROM fridge_items WHERE user_id = ? GROUP BY canonical_name ORDER BY latest DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(rows
        .into_iter()
        .map(|(name, product_id, _latest)| (name, product_id))
        .collect())
}
