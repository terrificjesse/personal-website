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

use chrono::{TimeDelta, Utc};
use serde::Serialize;
use std::collections::HashMap;

use crate::{
    models::{FridgeItem, PurchaseHistory},
    recommend::SuggestionReason::{ExpiringReplacement, FrequentlyPurchased},
};

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

fn calculate_mad(item: &[&PurchaseHistory]) -> Option<TimeDelta> {
    let iterate = item.windows(2);
    let mut consec_median: Vec<TimeDelta> = iterate
        .map(|w| w[1].purchased_at - w[0].purchased_at)
        .collect();
    consec_median.sort();
    consec_median.get(consec_median.len() / 2).copied()
}

/// Suggests items to add to the shopping list from purchase history and current fridge
/// contents. See the module doc for the two signals PLAN.md names, and the module
/// boundary — this function's body is what you implement.
pub fn suggest_shopping_items(
    history: &[PurchaseHistory],
    fridge: &[FridgeItem],
) -> Vec<Suggestion> {
    let expiring_soon_cutoff = Utc::now() + TimeDelta::days(3);

    let mut suggest: Vec<Suggestion> = fridge
        .iter()
        .filter(|item| {
            // `estimated_expiration` is `None` when the item has no shelf-life estimate at
            // all — that's "unknown," not "expiring soon," so it must not match here.
            // Relying on `Option`'s default ordering would get this backwards: `None` sorts
            // as less than every `Some(_)`, so a plain `<` comparison would treat "no
            // estimate" as always expiring before the cutoff.
            item.estimated_expiration
                .is_some_and(|expiration| expiration < expiring_soon_cutoff)
        })
        .map(|item| Suggestion {
            item_name: item.canonical_name.clone(),
            reason: ExpiringReplacement,
        })
        .collect();

    let mut by_item: HashMap<&str, Vec<&PurchaseHistory>> = HashMap::new();
    for entry in history {
        by_item
            .entry(entry.item_name.as_str())
            .or_default()
            .push(entry)
    }
    for item in by_item.values_mut() {
        item.sort_by_key(|p| p.purchased_at);
    }

    for (key, item) in &by_item {
        if !fridge.iter().any(|f| f.canonical_name == *key)
            && let Some(mad) = calculate_mad(item)
            && mad < Utc::now() - item.last().unwrap().purchased_at
        {
            suggest.push(Suggestion {
                item_name: key.to_string(),
                reason: FrequentlyPurchased,
            });
        }
    }
    suggest
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

        assert!(suggests(
            &suggestions,
            "milk",
            SuggestionReason::FrequentlyPurchased
        ));
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

        assert!(!suggests(
            &suggestions,
            "milk",
            SuggestionReason::FrequentlyPurchased
        ));
    }

    #[test]
    fn suggests_replacement_for_item_expiring_within_two_days() {
        let history = vec![];
        let fridge = vec![fridge_item(
            "spinach",
            Some(Utc::now() + Duration::hours(36)),
        )];

        let suggestions = suggest_shopping_items(&history, &fridge);

        assert!(suggests(
            &suggestions,
            "spinach",
            SuggestionReason::ExpiringReplacement
        ));
    }

    #[test]
    fn does_not_suggest_replacement_for_item_with_plenty_of_shelf_life_left() {
        let history = vec![];
        let fridge = vec![fridge_item(
            "spinach",
            Some(Utc::now() + Duration::days(10)),
        )];

        let suggestions = suggest_shopping_items(&history, &fridge);

        assert!(!suggests(
            &suggestions,
            "spinach",
            SuggestionReason::ExpiringReplacement
        ));
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

        assert!(!suggests(
            &suggestions,
            "saffron",
            SuggestionReason::FrequentlyPurchased
        ));
    }
}
