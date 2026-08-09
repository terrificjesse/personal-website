use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::expiration::estimate_expiration;
use crate::models::{AddItemRequest, FridgeItem};
use crate::purchase_history;

/// How far apart two expiration dates can be and still count as the same batch.
///
/// Two rows for the same food normally differ in expiration by however far apart they were
/// added, so this is effectively "bought within a few days of each other". Widen it and
/// week-old milk absorbs today's; narrow it and you get a new row every trip.
const MERGE_EXPIRATION_TOLERANCE_DAYS: i64 = 3;

pub async fn list_items(
    State(pool): State<SqlitePool>,
) -> Result<Json<Vec<FridgeItem>>, StatusCode> {
    Ok(Json(fetch_all(&pool).await?))
}

/// All fridge items, most recently added first. Shared by `GET /items` and the
/// shopping-list suggestions endpoint, which needs current fridge contents as input to
/// `recommend::suggest_shopping_items`.
pub(crate) async fn fetch_all(pool: &SqlitePool) -> Result<Vec<FridgeItem>, StatusCode> {
    sqlx::query_as::<_, FridgeItem>(
        "SELECT id, canonical_name, quantity, unit, added_at, estimated_expiration, \
         foodkeeper_product_id \
         FROM fridge_items ORDER BY added_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn add_item(
    State(pool): State<SqlitePool>,
    Json(req): Json<AddItemRequest>,
) -> Result<(StatusCode, Json<FridgeItem>), StatusCode> {
    let (status, item) = upsert_fridge_item(&pool, &req).await?;
    Ok((status, Json(item)))
}

/// Inserts a new fridge row for `req`, or merges into an existing one when
/// `find_merge_target` says they're the same batch. This is the single call site for
/// "a grocery item was acquired" — it's used directly by `POST /items` and also by
/// `shopping_list::mark_purchased` when a grocery item is checked off the shopping list,
/// so a purchase is logged to `purchase_history` exactly once no matter which flow
/// produced it, never twice.
pub(crate) async fn upsert_fridge_item(
    pool: &SqlitePool,
    req: &AddItemRequest,
) -> Result<(StatusCode, FridgeItem), StatusCode> {
    // Name resolution happens before the request gets here: the user picks from the
    // suggestion dropdown (`GET /items/suggest`) or commits what they typed. The server
    // takes the confirmed name at face value and only normalizes whitespace/casing.
    let canonical_name = req.name.trim().to_lowercase();
    if canonical_name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let added_at = Utc::now();
    let estimated_expiration = estimate_expiration(&canonical_name, added_at);

    // Same name and unit only makes a row *eligible* to merge — the expiration still has to
    // line up, or adding fresh milk would silently join a carton that's about to turn.
    // Units must match too: 2 count + 1 litre is not 3 of anything.
    let existing = sqlx::query_as::<_, MergeCandidate>(
        "SELECT id, estimated_expiration FROM fridge_items \
         WHERE canonical_name = ? AND unit = ?",
    )
    .bind(&canonical_name)
    .bind(&req.unit)
    .fetch_all(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if let Some(target) = find_merge_target(&existing, estimated_expiration) {
        let result = merge_into_existing(pool, target, req, estimated_expiration).await?;
        purchase_history::record(pool, &canonical_name, req.quantity, added_at)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(result);
    }

    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO fridge_items (id, canonical_name, quantity, unit, added_at, estimated_expiration, foodkeeper_product_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&canonical_name)
    .bind(req.quantity)
    .bind(&req.unit)
    .bind(added_at)
    .bind(estimated_expiration)
    .bind(req.foodkeeper_product_id)
    .execute(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    purchase_history::record(pool, &canonical_name, req.quantity, added_at)
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

/// An existing row that `add_item` could fold the new quantity into.
#[derive(Debug, Clone, sqlx::FromRow)]
struct MergeCandidate {
    id: String,
    estimated_expiration: Option<DateTime<Utc>>,
}

/// Picks the row a newly added item should merge into, if any.
///
/// Kept separate from the SQL so the decision is unit-testable. Rows without an expiration
/// are never merge targets — there's nothing to compare against, so treating them as "close
/// enough" would be a guess. When several rows qualify, the earliest-expiring one wins, so
/// the oldest open batch is the one that grows.
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

/// Folds the new quantity into an existing row and returns the updated item.
async fn merge_into_existing(
    pool: &SqlitePool,
    target: &MergeCandidate,
    req: &AddItemRequest,
    new_expiration: DateTime<Utc>,
) -> Result<(StatusCode, FridgeItem), StatusCode> {
    // Keep the *earlier* of the two dates. The merged row now covers food of slightly
    // different ages, and warning early about food that's still fine is a much cheaper
    // mistake than staying quiet about food that isn't.
    let expiration = target
        .estimated_expiration
        .map_or(new_expiration, |existing| existing.min(new_expiration));

    sqlx::query(
        "UPDATE fridge_items \
         SET quantity = quantity + ?, \
             estimated_expiration = ?, \
             foodkeeper_product_id = COALESCE(foodkeeper_product_id, ?) \
         WHERE id = ?",
    )
    .bind(req.quantity)
    .bind(expiration)
    // Backfills the catalog id if the existing row was added freehand and this one came
    // from a suggestion. COALESCE keeps an id the row already had.
    .bind(req.foodkeeper_product_id)
    .bind(&target.id)
    .execute(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let item = sqlx::query_as::<_, FridgeItem>(
        "SELECT id, canonical_name, quantity, unit, added_at, estimated_expiration, \
         foodkeeper_product_id \
         FROM fridge_items WHERE id = ?",
    )
    .bind(&target.id)
    .fetch_one(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 200 rather than 201: this updated a row instead of creating one.
    Ok((StatusCode::OK, item))
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
        // Milk bought two weeks ago should not absorb milk bought today.
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
            Some(new_expiration - Duration::days(MERGE_EXPIRATION_TOLERANCE_DAYS) - Duration::minutes(1)),
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
