//! Reads and writes for the `purchase_history` table.
//!
//! Writes happen from exactly one place — `routes::items::upsert_fridge_item` — so a
//! purchase is logged once whether it came from the add-item form or from marking a
//! shopping-list item purchased. See that function's doc comment.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::PurchaseHistory;

pub async fn record(
    pool: &SqlitePool,
    user_id: &str,
    item_name: &str,
    quantity: f64,
    purchased_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO purchase_history (id, item_name, quantity, purchased_at, user_id) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(item_name)
    .bind(quantity)
    .bind(purchased_at)
    .bind(user_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// One account's purchase history, most recent first. Feeds
/// `recommend::suggest_shopping_items`.
///
/// Scoping matters more here than it looks: `suggest_shopping_items` reads *frequency and
/// recency* out of this history, so pooling two accounts wouldn't merely show the wrong rows
/// — it would silently distort the purchase intervals the whole suggestion heuristic is
/// built on.
pub async fn list_for_user(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<PurchaseHistory>, sqlx::Error> {
    sqlx::query_as::<_, PurchaseHistory>(
        "SELECT id, item_name, quantity, purchased_at \
         FROM purchase_history WHERE user_id = ? ORDER BY purchased_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}
