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
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct FoodKeeperRow {
    Name: String,

    Pantry_Max: Option<u32>,
    Pantry_Metric: Option<String>,

    DOP_Pantry_Max: Option<u32>,
    DOP_Pantry_Metric: Option<String>,

    Refrigerate_Max: Option<u32>,
    Refrigerate_Metric: Option<String>,

    DOP_Refrigerate_Max: Option<u32>,
    DOP_Refrigerate_Metric: Option<String>,

    Refrigerate_After_Opening_Max: Option<u32>,
    Refrigerate_After_Opening_Metric: Option<String>,

    Freeze_Max: Option<u32>,
    Freeze_Metric: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Storage {
    FridgeFromPurchase,
    FridgeAfterOpening,
    FridgeAfterDate,
    PantryFromPurchase,
    PantryAfterDate,
    Freezer,
}

impl FoodKeeperRow {
    fn storage_options(&self) -> [(Storage, Option<u32>, Option<&str>); 6] {
        [
            (
                Storage::FridgeFromPurchase,
                self.DOP_Refrigerate_Max,
                self.DOP_Refrigerate_Metric.as_deref(),
            ),
            (
                Storage::FridgeAfterOpening,
                self.Refrigerate_After_Opening_Max,
                self.Refrigerate_After_Opening_Metric.as_deref(),
            ),
            (
                Storage::FridgeAfterDate,
                self.Refrigerate_Max,
                self.Refrigerate_Metric.as_deref(),
            ),
            (
                Storage::PantryFromPurchase,
                self.DOP_Pantry_Max,
                self.DOP_Pantry_Metric.as_deref(),
            ),
            (
                Storage::PantryAfterDate,
                self.Pantry_Max,
                self.Pantry_Metric.as_deref(),
            ),
            (
                Storage::Freezer,
                self.Freeze_Max,
                self.Freeze_Metric.as_deref(),
            ),
        ]
    }
    fn best_storage(&self) -> Option<(Storage, Option<u32>, Option<&str>)> {
        self.storage_options()
            .into_iter()
            .find(|(_, max, metric)| max.is_some() || metric.is_some())
    }
}

const PRODUCTS_CSV: &str = include_str!("../data/foodkeeper/products.csv");

fn parse_foodkeeper() -> Result<Vec<FoodKeeperRow>, csv::Error> {
    let mut rdr = csv::Reader::from_reader(PRODUCTS_CSV.as_bytes());
    rdr.deserialize().collect()
}

pub fn estimate_expiration(item_name: &str, added_at: DateTime<Utc>) -> DateTime<Utc> {
    let mut time = Duration::days(7);
    let food_data = parse_foodkeeper().expect("parse");
    for row in food_data {
        let borrow = &row;
        if borrow.Name.trim().eq_ignore_ascii_case(item_name) {
            if let Some((_, Some(num), Some(metric))) = row.best_storage() {
                match metric {
                    "Hours" => time = Duration::hours(num.into()),
                    "Days" => time = Duration::days(num.into()),
                    "Weeks" => time = Duration::weeks(num.into()),
                    "Months" => time = Duration::days((30 * num).into()),
                    "Years" => time = Duration::days((365 * num).into()),
                    "Year" => time = Duration::days((365 * num).into()),
                    _ => (),
                }
            }
            break;
        }
    }
    added_at + time
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
