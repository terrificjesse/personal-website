use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::{AddItemRequest, AddShoppingListItemRequest, ShoppingListItem, ShoppingListStatus};
use crate::purchase_history;
use crate::recommend::{self, Suggestion};
use crate::routes::items;

const SELECT_COLUMNS: &str = "id, name, quantity, unit, is_grocery, added_manually, status, \
     foodkeeper_product_id, added_at";

pub async fn list_shopping_list(
    State(pool): State<SqlitePool>,
) -> Result<Json<Vec<ShoppingListItem>>, StatusCode> {
    let items = sqlx::query_as::<_, ShoppingListItem>(&format!(
        "SELECT {SELECT_COLUMNS} FROM shopping_list_items ORDER BY added_at DESC"
    ))
    .fetch_all(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(items))
}

pub async fn add_shopping_list_item(
    State(pool): State<SqlitePool>,
    Json(req): Json<AddShoppingListItemRequest>,
) -> Result<(StatusCode, Json<ShoppingListItem>), StatusCode> {
    let name = req.name.trim().to_lowercase();
    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let id = Uuid::new_v4().to_string();
    let added_at = Utc::now();
    let status = ShoppingListStatus::Pending;

    sqlx::query(
        "INSERT INTO shopping_list_items \
         (id, name, quantity, unit, is_grocery, added_manually, status, foodkeeper_product_id, added_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(req.quantity)
    .bind(&req.unit)
    .bind(req.is_grocery)
    .bind(req.added_manually)
    .bind(status)
    .bind(req.foodkeeper_product_id)
    .bind(added_at)
    .execute(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let item = ShoppingListItem {
        id,
        name,
        quantity: req.quantity,
        unit: req.unit,
        is_grocery: req.is_grocery,
        added_manually: req.added_manually,
        status,
        foodkeeper_product_id: req.foodkeeper_product_id,
        added_at,
    };

    Ok((StatusCode::CREATED, Json(item)))
}

pub async fn remove_shopping_list_item(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let result = sqlx::query("DELETE FROM shopping_list_items WHERE id = ?")
        .bind(&id)
        .execute(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Marks a shopping-list row purchased. For grocery items this also folds the item into
/// the fridge via `items::upsert_fridge_item` — the same insert/merge path `POST /items`
/// uses — which is where the purchase gets logged to `purchase_history`. Non-grocery items
/// (paper towels, etc.) only flip status; they never touch the fridge table or purchase
/// history, per PLAN.md's Phase 2 scope.
pub async fn mark_purchased(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<ShoppingListItem>, StatusCode> {
    let item = sqlx::query_as::<_, ShoppingListItem>(&format!(
        "SELECT {SELECT_COLUMNS} FROM shopping_list_items WHERE id = ?"
    ))
    .bind(&id)
    .fetch_optional(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;

    if item.is_grocery {
        let add_req = AddItemRequest {
            name: item.name.clone(),
            quantity: item.quantity,
            unit: item.unit.clone(),
            foodkeeper_product_id: item.foodkeeper_product_id,
        };
        items::upsert_fridge_item(&pool, &add_req).await?;
    }

    sqlx::query("UPDATE shopping_list_items SET status = ? WHERE id = ?")
        .bind(ShoppingListStatus::Purchased)
        .bind(&id)
        .execute(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let updated = sqlx::query_as::<_, ShoppingListItem>(&format!(
        "SELECT {SELECT_COLUMNS} FROM shopping_list_items WHERE id = ?"
    ))
    .bind(&id)
    .fetch_one(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(updated))
}

pub async fn suggestions(
    State(pool): State<SqlitePool>,
) -> Result<Json<Vec<Suggestion>>, StatusCode> {
    let history = purchase_history::list_all(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let fridge = items::fetch_all(&pool).await?;

    Ok(Json(recommend::suggest_shopping_items(&history, &fridge)))
}
