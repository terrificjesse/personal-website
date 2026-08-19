
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
    /// Indicator that a product is frequently purchased and may be purchased again
    FrequentlyPurchased,
    /// Indicator that a product in the fridge may need replacement soon
    ExpiringReplacement,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub item_name: String,
    pub reason: SuggestionReason,
}

/// Helper function to calculate MAD - Median Absolute Deviation, an outlier
/// robust variance calculation that helps weed out long gaps between product
/// purchases that may occur due to trips or other circumstances.
fn calculate_mad(item: &[&PurchaseHistory]) -> Option<TimeDelta> {
    let iterate = item.windows(2);
    let mut consec_median: Vec<TimeDelta> = iterate
        .map(|w| w[1].purchased_at - w[0].purchased_at)
        .collect();
    consec_median.sort();
    consec_median.get(consec_median.len() / 2).copied()
}

/// Compiles suggestable items based on items in the fridge and items frequently
/// purchased. Items in the fridge that are expiring within 3 days are added first.
/// Then the items in purchased history are hashed and sorted by the date they were
/// purchased. The variance in purchase times for each item not already in the
/// fridge is calculated and if it's due for a purchase it will be added to the
/// suggestions.
pub fn suggest_shopping_items(
    history: &[PurchaseHistory],
    fridge: &[FridgeItem],
) -> Vec<Suggestion> {
    let expiring_soon_cutoff = Utc::now() + TimeDelta::days(3);

    let mut suggest: Vec<Suggestion> = fridge
        .iter()
        .filter(|item| {
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
