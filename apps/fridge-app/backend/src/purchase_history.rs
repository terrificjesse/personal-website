
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::PurchaseHistory;

// Adds a row to the Purchase History database
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

// Returns the Purchase History for a specific user with user_id:
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
