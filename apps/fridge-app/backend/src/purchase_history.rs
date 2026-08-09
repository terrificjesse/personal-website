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
    item_name: &str,
    quantity: f64,
    purchased_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO purchase_history (id, item_name, quantity, purchased_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(item_name)
    .bind(quantity)
    .bind(purchased_at)
    .execute(pool)
    .await?;

    Ok(())
}

/// All purchase history, most recent first. Feeds `recommend::suggest_shopping_items`.
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<PurchaseHistory>, sqlx::Error> {
    sqlx::query_as::<_, PurchaseHistory>(
        "SELECT id, item_name, quantity, purchased_at \
         FROM purchase_history ORDER BY purchased_at DESC",
    )
    .fetch_all(pool)
    .await
}
