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

const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 25;

#[derive(Debug, Deserialize)]
pub struct SuggestQuery {
    #[serde(default)]
    pub q: String,
    pub limit: Option<usize>,
}

/// `GET /items/suggest?q=&limit=` — ranked name suggestions for the add-item dropdown.
///
/// An empty `q` returns recently added fridge items rather than running the ranker.
pub async fn suggest_items(
    State(pool): State<SqlitePool>,
    State(catalog): State<Arc<Catalog>>,
    Query(params): Query<SuggestQuery>,
) -> Result<Json<Vec<Suggestion>>, StatusCode> {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let query = params.q.trim();

    let fridge_names = fetch_fridge_names(&pool).await?;

    if query.is_empty() {
        // Empty-query state: most recently added items. Most fridge additions are repeats,
        // so this is the highest-value thing to show before a key is pressed.
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

    // Index the fridge candidates by name so catalog entries can be folded *into* them
    // rather than appended alongside. Without this, anything that's both in the fridge and
    // in FoodKeeper (eggs, ham, milk) yields two rows that match identically and score
    // identically — a visible duplicate in the dropdown.
    //
    // Keys are cloned rather than borrowed because the loop below mutates `candidates`,
    // and a borrowed key would hold an immutable borrow across that mutation.
    let fridge_by_name: HashMap<String, usize> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| (candidate.name_lower.clone(), index))
        .collect();

    for entry in catalog.entries() {
        match fridge_by_name.get(&entry.name_lower) {
            // Already in the fridge. Merge instead of dropping either side: the fridge row
            // knows it's in the fridge, the catalog row knows the synonyms and the product
            // id. Keeping `SuggestionSource::Fridge` means it still renders "in fridge" and
            // still routes to the quantity-merge path in `add_item`.
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
                // Representative row for names that collapse several CSV rows — see
                // `CatalogEntry::product_ids`.
                foodkeeper_product_id: entry.product_ids.first().copied(),
            }),
        }
    }

    Ok(Json(suggest_item_names(query, &candidates, limit)))
}

/// Distinct fridge item names, most recently added first, each carrying the FoodKeeper id
/// it was last added with (if any).
async fn fetch_fridge_names(pool: &SqlitePool) -> Result<Vec<(String, Option<i64>)>, StatusCode> {
    // SQLite guarantees that with a bare MAX() aggregate, the non-aggregated columns come
    // from the row that produced the maximum — so this yields each name's newest row.
    // `added_at` is stored as RFC 3339 TEXT, which sorts correctly lexicographically.
    let rows = sqlx::query_as::<_, (String, Option<i64>, String)>(
        "SELECT canonical_name, foodkeeper_product_id, MAX(added_at) AS latest \
         FROM fridge_items GROUP BY canonical_name ORDER BY latest DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(rows
        .into_iter()
        .map(|(name, product_id, _latest)| (name, product_id))
        .collect())
}
