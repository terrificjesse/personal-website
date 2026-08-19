use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::expiration::estimate_expiration;
use crate::models::{AddItemRequest, FridgeItem};
use crate::purchase_history;
use crate::routes::auth::CurrentUser;

const SELECT_COLUMNS: &str = "id, canonical_name, quantity, unit, added_at, \
     estimated_expiration, foodkeeper_product_id";

/// The margin of error in which items will be merged:
const MERGE_EXPIRATION_TOLERANCE_DAYS: i64 = 3;

pub async fn list_items(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<FridgeItem>>, StatusCode> {
    Ok(Json(fetch_all(&pool, &user.id).await?))
}

/// Grabs all of the items in the fridge for a given user
pub(crate) async fn fetch_all(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<FridgeItem>, StatusCode> {
    sqlx::query_as::<_, FridgeItem>(&format!(
        "SELECT {SELECT_COLUMNS} FROM fridge_items WHERE user_id = ? ORDER BY added_at DESC"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn add_item(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AddItemRequest>,
) -> Result<(StatusCode, Json<FridgeItem>), StatusCode> {
    let (status, item) = upsert_fridge_item(&pool, &user.id, &req).await?;
    Ok((status, Json(item)))
}

/// Adds an item to the pool:
pub(crate) async fn upsert_fridge_item(
    pool: &SqlitePool,
    user_id: &str,
    req: &AddItemRequest,
) -> Result<(StatusCode, FridgeItem), StatusCode> {
    // Normalize the name:
    let canonical_name = req.name.trim().to_lowercase();
    if canonical_name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let added_at = Utc::now();
    let estimated_expiration = estimate_expiration(&canonical_name, added_at);

    // Fetches all items in the fridge currently matching the specs of the item requesting to be added:
    let existing = sqlx::query_as::<_, MergeCandidate>(
        "SELECT id, estimated_expiration FROM fridge_items \
         WHERE user_id = ? AND canonical_name = ? AND unit = ?",
    )
    .bind(user_id)
    .bind(&canonical_name)
    .bind(&req.unit)
    .fetch_all(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Finds a suitable matching item from the fetched list and merges it into the fridge
    if let Some(target) = find_merge_target(&existing, estimated_expiration) {
        let result = merge_into_existing(pool, target, req, estimated_expiration).await?;
        purchase_history::record(pool, user_id, &canonical_name, req.quantity, added_at)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(result);
    }

    let id = Uuid::new_v4().to_string();

    // Insertion into the fridge database
    sqlx::query(
        "INSERT INTO fridge_items (id, canonical_name, quantity, unit, added_at, estimated_expiration, foodkeeper_product_id, user_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&canonical_name)
    .bind(req.quantity)
    .bind(&req.unit)
    .bind(added_at)
    .bind(estimated_expiration)
    .bind(req.foodkeeper_product_id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Records the item as purchased
    purchase_history::record(pool, user_id, &canonical_name, req.quantity, added_at)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let item = FridgeItem {
        id,
        canonical_name,
        quantity: req.quantity,
        unit: req.unit.clone(),
        added_at,
        estimated_expiration: Some(estimated_expiration),
        foodkeeper_product_id: req.foodkeeper_product_id,
    };

    Ok((StatusCode::CREATED, item))
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct MergeCandidate {
    id: String,
    estimated_expiration: Option<DateTime<Utc>>,
}

/// This is a Helper function that finds a matching candidate with an expiration
/// date within the tolerance of the expiration date of the input target.
fn find_merge_target(
    candidates: &[MergeCandidate],
    new_expiration: DateTime<Utc>,
) -> Option<&MergeCandidate> {
    let tolerance = Duration::days(MERGE_EXPIRATION_TOLERANCE_DAYS);

    candidates
        .iter()
        .filter_map(|candidate| {
            candidate
                .estimated_expiration
                .map(|expiration| (candidate, expiration))
        })
        .filter(|(_, expiration)| (*expiration - new_expiration).abs() <= tolerance)
        .min_by_key(|(_, expiration)| *expiration)
        .map(|(candidate, _)| candidate)
}

/// Takes an input item and attempts to merge it with an existing entry in the
/// fridge table.
async fn merge_into_existing(
    pool: &SqlitePool,
    target: &MergeCandidate,
    req: &AddItemRequest,
    new_expiration: DateTime<Utc>,
) -> Result<(StatusCode, FridgeItem), StatusCode> {
    // Take the earlier time of the 2 expirations to ensure safety
    let expiration = target
        .estimated_expiration
        .map_or(new_expiration, |existing| existing.min(new_expiration));

    // Update query
    sqlx::query(
        "UPDATE fridge_items \
         SET quantity = quantity + ?, \
             estimated_expiration = ?, \
             foodkeeper_product_id = COALESCE(foodkeeper_product_id, ?) \
         WHERE id = ?",
    )
    .bind(req.quantity)
    .bind(expiration)
    .bind(req.foodkeeper_product_id)
    .bind(&target.id)
    .execute(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Takes the updated data from the merged row
    let item = sqlx::query_as::<_, FridgeItem>(&format!(
        "SELECT {SELECT_COLUMNS} FROM fridge_items WHERE id = ?"
    ))
    .bind(&target.id)
    .fetch_one(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((StatusCode::OK, item))
}

/// Removes an item from the fridge
pub async fn remove_item(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let result = sqlx::query("DELETE FROM fridge_items WHERE id = ? AND user_id = ?")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, expiration: Option<DateTime<Utc>>) -> MergeCandidate {
        MergeCandidate {
            id: id.to_string(),
            estimated_expiration: expiration,
        }
    }

    fn days_from_now(days: i64) -> DateTime<Utc> {
        Utc::now() + Duration::days(days)
    }

    #[test]
    fn merges_when_expirations_are_close() {
        let new_expiration = days_from_now(7);
        let existing = vec![candidate("a", Some(days_from_now(5)))];

        assert_eq!(
            find_merge_target(&existing, new_expiration).map(|c| c.id.as_str()),
            Some("a")
        );
    }

    #[test]
    fn does_not_merge_when_expirations_are_far_apart() {
        let new_expiration = days_from_now(7);
        let existing = vec![candidate("a", Some(days_from_now(-7)))];

        assert!(find_merge_target(&existing, new_expiration).is_none());
    }

    #[test]
    fn merges_across_the_tolerance_boundary_but_not_past_it() {
        let new_expiration = days_from_now(10);

        let inside = vec![candidate(
            "a",
            Some(new_expiration - Duration::days(MERGE_EXPIRATION_TOLERANCE_DAYS)),
        )];
        assert!(find_merge_target(&inside, new_expiration).is_some());

        let outside = vec![candidate(
            "b",
            Some(
                new_expiration
                    - Duration::days(MERGE_EXPIRATION_TOLERANCE_DAYS)
                    - Duration::minutes(1),
            ),
        )];
        assert!(find_merge_target(&outside, new_expiration).is_none());
    }

    #[test]
    fn tolerance_applies_in_both_directions() {
        let new_expiration = days_from_now(10);
        let later = vec![candidate("a", Some(new_expiration + Duration::days(2)))];

        assert!(find_merge_target(&later, new_expiration).is_some());
    }

    #[test]
    fn picks_the_earliest_expiring_match() {
        let new_expiration = days_from_now(7);
        let existing = vec![
            candidate("later", Some(days_from_now(8))),
            candidate("earliest", Some(days_from_now(5))),
            candidate("middle", Some(days_from_now(6))),
        ];

        assert_eq!(
            find_merge_target(&existing, new_expiration).map(|c| c.id.as_str()),
            Some("earliest")
        );
    }

    #[test]
    fn rows_without_an_expiration_are_never_merge_targets() {
        let existing = vec![candidate("a", None)];

        assert!(find_merge_target(&existing, days_from_now(7)).is_none());
    }

    #[test]
    fn no_candidates_means_no_merge() {
        assert!(find_merge_target(&[], days_from_now(7)).is_none());
    }
}
