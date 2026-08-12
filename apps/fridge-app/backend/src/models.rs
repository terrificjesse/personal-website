use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct FridgeItem {
    // Stored as TEXT in sqlite; kept as String rather than uuid::Uuid to avoid
    // relying on sqlx's BLOB-based Uuid encoding, which doesn't match our TEXT column.
    pub id: String,
    pub canonical_name: String,
    pub quantity: f64,
    pub unit: String,
    pub added_at: DateTime<Utc>,
    pub estimated_expiration: Option<DateTime<Utc>>,
    /// Which FoodKeeper product the user picked from the suggestion dropdown, if any.
    /// `None` means the name was typed freehand.
    pub foodkeeper_product_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AddItemRequest {
    pub name: String,
    #[serde(default = "default_quantity")]
    pub quantity: f64,
    #[serde(default = "default_unit")]
    pub unit: String,
    /// Set when the name came from a suggestion; omitted for freehand entries.
    #[serde(default)]
    pub foodkeeper_product_id: Option<i64>,
}

fn default_quantity() -> f64 {
    1.0
}

fn default_unit() -> String {
    "count".to_string()
}

fn default_is_grocery() -> bool {
    true
}

fn default_added_manually() -> bool {
    true
}

/// Pending vs. purchased. Stored as TEXT in sqlite (`sqlx::Type` maps the variant name
/// directly, same trick as `SuggestionSource` in `nlp.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ShoppingListStatus {
    Pending,
    Purchased,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ShoppingListItem {
    pub id: String,
    pub name: String,
    pub quantity: f64,
    pub unit: String,
    /// Excludes the row from purchase history and from `suggest_shopping_items`'s input —
    /// see PLAN.md Phase 2. Non-grocery items (paper towels, etc.) never touch the fridge
    /// table either, since `mark_purchased` only calls `items::upsert_fridge_item` when
    /// this is true.
    pub is_grocery: bool,
    /// False when this row was created by accepting a suggestion rather than typed in —
    /// purely informational for now, not read by any query.
    pub added_manually: bool,
    pub status: ShoppingListStatus,
    pub foodkeeper_product_id: Option<i64>,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct AddShoppingListItemRequest {
    pub name: String,
    #[serde(default = "default_quantity")]
    pub quantity: f64,
    #[serde(default = "default_unit")]
    pub unit: String,
    #[serde(default = "default_is_grocery")]
    pub is_grocery: bool,
    #[serde(default = "default_added_manually")]
    pub added_manually: bool,
    #[serde(default)]
    pub foodkeeper_product_id: Option<i64>,
}

/// One grocery acquisition event. Logged in exactly one place — `items::upsert_fridge_item`
/// — regardless of whether it was triggered by the add-item form or by marking a
/// shopping-list item purchased, so a purchase is never double-counted. See
/// `apps/fridge-app/CLAUDE.md` for why that trigger was chosen over logging at both call
/// sites independently.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PurchaseHistory {
    pub id: String,
    pub item_name: String,
    pub quantity: f64,
    pub purchased_at: DateTime<Utc>,
}

/// One ingredient line from a recipe — a name plus its free-text measure ("2 cloves", "1
/// cup"). Not parsed into a quantity/unit pair; TheMealDB's measures aren't consistent
/// enough to parse reliably (see `data/themealdb/README.md`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeIngredient {
    pub name: String,
    pub measure: String,
}

/// A recipe from the vendored TheMealDB catalog (`src/themealdb.rs`). Static reference
/// data, not a DB row — there's no `sqlx::FromRow` here because these are never queried
/// from `fridge.db`; they're parsed once at startup from `data/themealdb/meals.json`.
///
/// `id` is TheMealDB's `idMeal`, kept as the source gave it (a numeric string) rather than
/// parsed to an integer — same reasoning as every other `id` field in this file: nothing
/// here does arithmetic on it, and Phase 4's `Review.recipe_id` just needs a stable key to
/// reference back to.
///
/// See `data/themealdb/README.md` for how each field below was derived from the source
/// data, including which ones are heuristics rather than structured facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub cuisine_tags: Vec<String>,
    pub meal_type_tags: Vec<String>,
    /// Always `None` — TheMealDB has no cook-time field and nothing else in the source data
    /// is reliable enough to derive one from. See `data/themealdb/README.md`.
    pub cook_time_minutes: Option<u32>,
    /// Best-effort, keyword-derived. See `APPLIANCE_KEYWORDS` in `src/themealdb.rs`.
    pub required_appliances: Vec<String>,
    /// Ingredients an inventory app would plausibly track (proteins, produce, dairy, etc).
    pub fridge_ingredients: Vec<RecipeIngredient>,
    /// Pantry staples/spices, split out via a keyword list rather than tracked individually.
    pub extra_ingredients: Vec<RecipeIngredient>,
    pub image_url: Option<String>,
}
