use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::foodkeeper::Catalog;
use crate::nlp::{suggest_item_names, Candidate, Suggestion, SuggestionSource};

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
            name,
            source: SuggestionSource::Fridge,
            aliases: Vec::new(),
            foodkeeper_product_id,
        })
        .collect();

    candidates.extend(catalog.entries().iter().map(|entry| Candidate {
        name: entry.name.clone(),
        source: SuggestionSource::Foodkeeper,
        aliases: entry.aliases.clone(),
        // Representative row for names that collapse several CSV rows — see
        // `CatalogEntry::product_ids`.
        foodkeeper_product_id: entry.product_ids.first().copied(),
    }));

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
