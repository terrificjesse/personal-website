use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::expiration::estimate_expiration;
use crate::models::{AddItemRequest, FridgeItem};
use crate::nlp::{resolve_item_name, MatchResult};

pub async fn list_items(
    State(pool): State<SqlitePool>,
) -> Result<Json<Vec<FridgeItem>>, StatusCode> {
    let items = sqlx::query_as::<_, FridgeItem>(
        "SELECT id, canonical_name, quantity, unit, added_at, estimated_expiration \
         FROM fridge_items ORDER BY added_at DESC",
    )
    .fetch_all(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(items))
}

pub async fn add_item(
    State(pool): State<SqlitePool>,
    Json(req): Json<AddItemRequest>,
) -> Result<(StatusCode, Json<FridgeItem>), StatusCode> {
    let existing: Vec<String> = sqlx::query_scalar("SELECT canonical_name FROM fridge_items")
        .fetch_all(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let canonical_name = match resolve_item_name(&req.name, &existing) {
        MatchResult::Exact(name) | MatchResult::Fuzzy(name) => name,
        MatchResult::NoMatch => req.name.trim().to_lowercase(),
    };

    let id = Uuid::new_v4().to_string();
    let added_at = Utc::now();
    let estimated_expiration = estimate_expiration(&canonical_name, added_at);

    sqlx::query(
        "INSERT INTO fridge_items (id, canonical_name, quantity, unit, added_at, estimated_expiration) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&canonical_name)
    .bind(req.quantity)
    .bind(&req.unit)
    .bind(added_at)
    .bind(estimated_expiration)
    .execute(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let item = FridgeItem {
        id,
        canonical_name,
        quantity: req.quantity,
        unit: req.unit,
        added_at,
        estimated_expiration: Some(estimated_expiration),
    };

    Ok((StatusCode::CREATED, Json(item)))
}

pub async fn remove_item(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let result = sqlx::query("DELETE FROM fridge_items WHERE id = ?")
        .bind(&id)
        .execute(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(StatusCode::NO_CONTENT)
}
