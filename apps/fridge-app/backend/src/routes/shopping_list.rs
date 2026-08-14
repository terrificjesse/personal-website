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

/// One account's shopping-list rows, most recently added first. Shared by
/// `GET /shopping-list` and the recipe-recommendation endpoint, which needs current
/// shopping-list contents as input to `recommend_recipes::recommend_recipes` — same
/// reasoning, and same required `user_id`, as `items::fetch_all`.
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

/// A pending row `add_shopping_list_item` could fold a new quantity into.
#[derive(Debug, Clone, sqlx::FromRow)]
struct MergeCandidate {
    id: String,
}

pub async fn add_shopping_list_item(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AddShoppingListItemRequest>,
) -> Result<(StatusCode, Json<ShoppingListItem>), StatusCode> {
    let name = req.name.trim().to_lowercase();
    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Same name and unit, still pending — same idea as the fridge's add-and-merge
    // (`items::upsert_fridge_item`), just without an expiration tolerance to worry about.
    // A purchased row never absorbs a new add: reviving a "done" row's quantity behind the
    // scenes would be more confusing than just starting a fresh pending row for it.
    //
    // Scoped by `user_id` for the same reason `items::upsert_fridge_item`'s merge query is —
    // merging across accounts would fold one person's row into another's destructively.
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

    if let Some(target) = existing {
        sqlx::query(
            "UPDATE shopping_list_items \
             SET quantity = quantity + ?, foodkeeper_product_id = COALESCE(foodkeeper_product_id, ?) \
             WHERE id = ?",
        )
        .bind(req.quantity)
        // Backfills the catalog id if the existing row was freehand and this add wasn't.
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

        // 200 rather than 201: this updated a row instead of creating one.
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
    // Ownership in the WHERE clause — see `items::remove_item` for why this is a filter
    // rather than a check-then-delete.
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

/// Marks a shopping-list row purchased. For grocery items this also folds the item into
/// the fridge via `items::upsert_fridge_item` — the same insert/merge path `POST /items`
/// uses — which is where the purchase gets logged to `purchase_history`. Non-grocery items
/// (paper towels, etc.) only flip status; they never touch the fridge table or purchase
/// history, per PLAN.md's Phase 2 scope.
pub async fn mark_purchased(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<ShoppingListItem>, StatusCode> {
    // Scoped, so a row belonging to another account 404s here rather than being marked
    // purchased *and* folded into this caller's fridge below.
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
