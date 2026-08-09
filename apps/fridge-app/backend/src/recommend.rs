//! Shopping list recommendations — flagged as a learning area, see CLAUDE.md.
//!
//! Goal: given purchase history and the current fridge contents, suggest what to add to
//! the shopping list. PLAN.md names two signals to start from:
//!   - an item purchased on a roughly regular cadence that isn't currently in the fridge
//!     (a frequency/recency heuristic — you don't need ML for this)
//!   - a fridge item expiring soon, suggested as a replacement
//!
//! Non-grocery items never reach this function: `shopping_list::mark_purchased` only logs
//! purchase history for `is_grocery` rows, so `history` is grocery-only by construction.
//!
//! TODO(you): replace the body of `suggest_shopping_items`. The tests below describe the
//! required behavior — they will fail against the current placeholder (which always
//! returns an empty list) until you implement real scoring. Worth reading before you start:
//! simple frequency/recency heuristics (e.g. "days since last purchase" vs. "median gap
//! between purchases") as a legitimate first pass — see the "Working patterns established
//! in Phase 1" section of `apps/fridge-app/CLAUDE.md` for pitfalls that bit `nlp.rs` and
//! will likely bite this too (band overflows, unreachable branches, tests passing for the
//! wrong reason).

use serde::Serialize;

use crate::models::{FridgeItem, PurchaseHistory};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionReason {
    /// Purchased on a regular-enough cadence, and not currently in the fridge.
    FrequentlyPurchased,
    /// Already in the fridge, but expiring soon enough to suggest buying a replacement.
    ExpiringReplacement,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub item_name: String,
    pub reason: SuggestionReason,
}

/// Suggests items to add to the shopping list from purchase history and current fridge
/// contents. See the module doc for the two signals PLAN.md names, and the module
/// boundary — this function's body is what you implement.
pub fn suggest_shopping_items(
    _history: &[PurchaseHistory],
    _fridge: &[FridgeItem],
) -> Vec<Suggestion> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration, Utc};

    fn purchase(item_name: &str, days_ago: i64) -> PurchaseHistory {
        PurchaseHistory {
            id: "test".to_string(),
            item_name: item_name.to_string(),
            quantity: 1.0,
            purchased_at: Utc::now() - Duration::days(days_ago),
        }
    }

    fn fridge_item(name: &str, estimated_expiration: Option<DateTime<Utc>>) -> FridgeItem {
        FridgeItem {
            id: "test".to_string(),
            canonical_name: name.to_string(),
            quantity: 1.0,
            unit: "count".to_string(),
            added_at: Utc::now(),
            estimated_expiration,
            foodkeeper_product_id: None,
        }
    }

    fn suggests(suggestions: &[Suggestion], item_name: &str, reason: SuggestionReason) -> bool {
        suggestions
            .iter()
            .any(|s| s.item_name == item_name && s.reason == reason)
    }

    #[test]
    fn suggests_item_purchased_weekly_and_missing_from_fridge() {
        // Bought roughly every 7 days for the last month, and the fridge is empty — this
        // is the clearest possible "you're about to run out" signal.
        let history = vec![
            purchase("milk", 7),
            purchase("milk", 14),
            purchase("milk", 21),
            purchase("milk", 28),
        ];
        let fridge = vec![];

        let suggestions = suggest_shopping_items(&history, &fridge);

        assert!(suggests(&suggestions, "milk", SuggestionReason::FrequentlyPurchased));
    }

    #[test]
    fn does_not_suggest_frequently_purchased_item_already_in_fridge() {
        let history = vec![
            purchase("milk", 7),
            purchase("milk", 14),
            purchase("milk", 21),
            purchase("milk", 28),
        ];
        let fridge = vec![fridge_item("milk", Some(Utc::now() + Duration::days(10)))];

        let suggestions = suggest_shopping_items(&history, &fridge);

        assert!(!suggests(&suggestions, "milk", SuggestionReason::FrequentlyPurchased));
    }

    #[test]
    fn suggests_replacement_for_item_expiring_within_two_days() {
        let history = vec![];
        let fridge = vec![fridge_item("spinach", Some(Utc::now() + Duration::hours(36)))];

        let suggestions = suggest_shopping_items(&history, &fridge);

        assert!(suggests(&suggestions, "spinach", SuggestionReason::ExpiringReplacement));
    }

    #[test]
    fn does_not_suggest_replacement_for_item_with_plenty_of_shelf_life_left() {
        let history = vec![];
        let fridge = vec![fridge_item("spinach", Some(Utc::now() + Duration::days(10)))];

        let suggestions = suggest_shopping_items(&history, &fridge);

        assert!(!suggests(&suggestions, "spinach", SuggestionReason::ExpiringReplacement));
    }

    #[test]
    fn no_history_and_no_fridge_items_means_no_suggestions() {
        assert_eq!(suggest_shopping_items(&[], &[]), Vec::new());
    }

    #[test]
    fn one_off_purchase_is_not_a_frequency_signal() {
        // A single purchase three weeks ago isn't "regularly bought" by any reasonable
        // definition of cadence — this guards against a threshold so loose it fires on
        // one data point.
        let history = vec![purchase("saffron", 21)];
        let fridge = vec![];

        let suggestions = suggest_shopping_items(&history, &fridge);

        assert!(!suggests(&suggestions, "saffron", SuggestionReason::FrequentlyPurchased));
    }
}
