use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::{
    AddItemRequest, AddShoppingListItemRequest, ShoppingListItem, ShoppingListStatus,
};
use crate::purchase_history;
use crate::recommend::{self, Suggestion};
use crate::routes::auth::CurrentUser;
use crate::routes::items;

const SELECT_COLUMNS: &str = "id, name, quantity, unit, is_grocery, added_manually, status, \
     foodkeeper_product_id, added_at";

pub async fn list_shopping_list(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<ShoppingListItem>>, StatusCode> {
    Ok(Json(fetch_all(&pool, &user.id).await?))
}

// This function creates a query for all fridge rows accessible to the user with user_id:
pub(crate) async fn fetch_all(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<ShoppingListItem>, StatusCode> {
    sqlx::query_as::<_, ShoppingListItem>(&format!(
        "SELECT {SELECT_COLUMNS} FROM shopping_list_items \
         WHERE user_id = ? ORDER BY added_at DESC"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct MergeCandidate {
    id: String,
}

// Attempts to find if the item already exists in the database for a given user.
// If so, merge with the existing product.
// If not, add a fresh ShoppingListItem row.
pub async fn add_shopping_list_item(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AddShoppingListItemRequest>,
) -> Result<(StatusCode, Json<ShoppingListItem>), StatusCode> {
    let name = req.name.trim().to_lowercase();
    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Matches the candidate name, user id, the pending status, and the unit type to an existing row in the database:
    let existing = sqlx::query_as::<_, MergeCandidate>(
        "SELECT id FROM shopping_list_items \
         WHERE user_id = ? AND name = ? AND unit = ? AND status = ? \
         ORDER BY added_at ASC LIMIT 1",
    )
    .bind(&user.id)
    .bind(&name)
    .bind(&req.unit)
    .bind(ShoppingListStatus::Pending)
    .fetch_optional(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // If a row matches, update the row to reflect the correct unit count
    if let Some(target) = existing {
        // COALESCE handles the case where a product is merged with and without the dropdown
        sqlx::query(
            "UPDATE shopping_list_items \
             SET quantity = quantity + ?, foodkeeper_product_id = COALESCE(foodkeeper_product_id, ?) \
             WHERE id = ?",
        )
        .bind(req.quantity)
        .bind(req.foodkeeper_product_id)
        .bind(&target.id)
        .execute(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let item = sqlx::query_as::<_, ShoppingListItem>(&format!(
            "SELECT {SELECT_COLUMNS} FROM shopping_list_items WHERE id = ?"
        ))
        .bind(&target.id)
        .fetch_one(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // successful merge and returns the merged row item:
        return Ok((StatusCode::OK, Json(item)));
    }

    let id = Uuid::new_v4().to_string();
    let added_at = Utc::now();
    let status = ShoppingListStatus::Pending;

    sqlx::query(
        "INSERT INTO shopping_list_items \
         (id, name, quantity, unit, is_grocery, added_manually, status, foodkeeper_product_id, added_at, user_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(&user.id)
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
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    // Matches the row id and user id to delete it:
    let result = sqlx::query("DELETE FROM shopping_list_items WHERE id = ? AND user_id = ?")
        .bind(&id)
        .bind(&user.id)
        .execute(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Matches the row id and user id to mark the product purchased:
pub async fn mark_purchased(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<ShoppingListItem>, StatusCode> {
    let item = sqlx::query_as::<_, ShoppingListItem>(&format!(
        "SELECT {SELECT_COLUMNS} FROM shopping_list_items WHERE id = ? AND user_id = ?"
    ))
    .bind(&id)
    .bind(&user.id)
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
        items::upsert_fridge_item(&pool, &user.id, &add_req).await?;
    }

    sqlx::query("UPDATE shopping_list_items SET status = ? WHERE id = ? AND user_id = ?")
        .bind(ShoppingListStatus::Purchased)
        .bind(&id)
        .bind(&user.id)
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

// Based on Purchase History and fridge contents, suggest shopping items
pub async fn suggestions(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<Suggestion>>, StatusCode> {
    let history = purchase_history::list_for_user(&pool, &user.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let fridge = items::fetch_all(&pool, &user.id).await?;

    Ok(Json(recommend::suggest_shopping_items(&history, &fridge)))
}
