//! Expiration date estimation — flagged as a learning area, see CLAUDE.md.
//!
//! Goal: given an item's canonical name and when it was added, estimate an expiration
//! date. A category → shelf-life lookup table (produce/dairy/meat/pantry + a fallback)
//! is a reasonable starting point; refine later if you want.
//!
//! TODO(you): replace the body of `estimate_expiration`. The tests below describe the
//! required behavior — they will fail against the current placeholder (which always
//! returns a flat 7 days) until you implement real category-based logic.

use chrono::{DateTime, Duration, Utc};

pub fn estimate_expiration(_item_name: &str, added_at: DateTime<Utc>) -> DateTime<Utc> {
    // Placeholder: flat 7-day shelf life for everything. Replace with real category
    // lookup (produce vs. dairy vs. meat vs. pantry, plus a sane fallback).
    added_at + Duration::days(7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produce_gets_a_short_shelf_life() {
        let added = Utc::now();
        let expires = estimate_expiration("lettuce", added);
        let days = (expires - added).num_days();
        assert!(
            (3..=10).contains(&days),
            "expected lettuce to expire in roughly 3-10 days, got {days}"
        );
    }

    #[test]
    fn pantry_items_get_a_long_shelf_life() {
        let added = Utc::now();
        let expires = estimate_expiration("rice", added);
        let days = (expires - added).num_days();
        assert!(
            days >= 60,
            "expected a pantry staple like rice to keep for months, got {days} days"
        );
    }

    #[test]
    fn dairy_gets_a_medium_shelf_life() {
        let added = Utc::now();
        let expires = estimate_expiration("milk", added);
        let days = (expires - added).num_days();
        assert!(
            (5..=21).contains(&days),
            "expected milk to expire in roughly 5-21 days, got {days}"
        );
    }

    #[test]
    fn unknown_item_gets_a_fallback_not_a_panic() {
        let added = Utc::now();
        let expires = estimate_expiration("some totally unknown food item", added);
        assert!(expires > added);
    }
}
