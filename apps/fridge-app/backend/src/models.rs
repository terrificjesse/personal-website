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
    /// Free-text cooking instructions straight from TheMealDB, trimmed but otherwise
    /// unprocessed. Quality varies with the source — most are a real paragraph or
    /// numbered-step list, but a few are as terse as "Make and enjoy" (see
    /// `data/themealdb/README.md`).
    pub instructions: String,
}

/// One rating a user gave a recipe after cooking it. `recipe_id` is TheMealDB's `idMeal`
/// (matches `Recipe.id`), not a foreign key — recipes are static reference data, not a DB
/// table. This is a history, not a single "current" review per recipe: re-cooking and
/// re-rating the same recipe over time is expected, and `GET /reviews` browses the whole
/// history rather than one row per recipe.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Review {
    pub id: String,
    pub recipe_id: String,
    /// 1–5. Whether a given rating counts as "liked" for the recommended-again section is
    /// `LIKED_RATING_THRESHOLD`, not baked into this struct — and whether/how a low rating
    /// suppresses a recipe elsewhere is `rerank_recommendations`'s job (`src/rerank.rs`).
    pub rating: i64,
    pub cooked_at: DateTime<Utc>,
    pub notes: Option<String>,
    /// Who wrote it. `None` for anything written before Phase 5 — the app was single-user,
    /// so there was nobody else it could belong to. See `Review::is_by`.
    pub user_id: Option<String>,
    /// Opt-in world-readability. Private reviews still count toward *your own*
    /// personalization; they're just never served to anyone else.
    pub is_public: bool,
    /// Moderation tombstone — excluded from every read path, but the row survives.
    pub hidden: bool,
}

impl Review {
    /// Whether `viewer` wrote this review — the personal-vs-global distinction
    /// `rerank_recommendations` needs in order to weight your own feedback differently from
    /// a stranger's.
    ///
    /// `viewer == None` means pre-Phase-5 single-user mode: there are no accounts yet, so
    /// every review in the database is by definition the local user's and counts as
    /// personal. Once Phase 5 threads a real session user id through, this becomes a genuine
    /// ownership check with no further changes at the call sites.
    pub fn is_by(&self, viewer: Option<&str>) -> bool {
        match viewer {
            None => true,
            Some(viewer_id) => self.user_id.as_deref() == Some(viewer_id),
        }
    }
}

/// A rating at or above this counts as "liked" for `GET /recipes/liked` — the plain
/// membership filter for the "recipes you liked" section (Phase 4). This is a simple
/// threshold, not the learned part of Phase 4; ordering *within* the liked set is
/// `rerank_recommendations`'s job.
pub const LIKED_RATING_THRESHOLD: i64 = 4;

/// A rating at or below this suppresses a recipe from the *general* recommendations
/// (`GET /recipes/recommended`), satisfying PLAN.md's Phase 4 checkpoint: "rate one poorly,
/// confirm it drops out of general recommendations."
///
/// Suppression deliberately lives on the general-recommendations path rather than inside
/// `rerank_recommendations` — a filter composes cleanly with Phase 3's ingredient ranking,
/// whereas a second *ordering* would fight it. `rerank_recommendations` only ever reorders.
///
/// No recency decay: a recipe you disliked stays suppressed regardless of when. Revisit if
/// permanently hiding something you disliked once starts feeling wrong (Phase 5's notes on
/// decay apply here too).
pub const SUPPRESSED_RATING_THRESHOLD: i64 = 2;

pub const MIN_RATING: i64 = 1;
pub const MAX_RATING: i64 = 5;

/// Cap on `Review.notes`. Nothing enforced this before; once reviews are world-readable an
/// unbounded free-text field reachable by `POST` is a liability, so the limit lands with the
/// schema rather than after it.
pub const MAX_NOTES_LENGTH: usize = 2_000;

#[derive(Debug, Deserialize)]
pub struct AddReviewRequest {
    pub recipe_id: String,
    pub rating: i64,
    #[serde(default)]
    pub notes: Option<String>,
    /// Defaults to submission time when omitted — most reviews are logged right after
    /// cooking.
    #[serde(default)]
    pub cooked_at: Option<DateTime<Utc>>,
    /// Opt-in. Defaults to private so a client that doesn't know about this field yet can
    /// never accidentally publish.
    #[serde(default)]
    pub is_public: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review(user_id: Option<&str>) -> Review {
        Review {
            id: "test".to_string(),
            recipe_id: "52967".to_string(),
            rating: 5,
            cooked_at: Utc::now(),
            notes: None,
            user_id: user_id.map(str::to_string),
            is_public: false,
            hidden: false,
        }
    }

    #[test]
    fn pre_auth_every_review_counts_as_personal() {
        // The configuration the app actually runs in today: no accounts, so `viewer` is None
        // and `user_id` is NULL on every row. Everything must read as the local user's, or
        // single-user personalization silently stops working.
        assert!(review(None).is_by(None));
        assert!(review(Some("someone")).is_by(None));
    }

    #[test]
    fn a_review_is_personal_only_to_its_own_author() {
        assert!(review(Some("me")).is_by(Some("me")));
        assert!(!review(Some("someone-else")).is_by(Some("me")));
    }

    #[test]
    fn a_pre_auth_review_belongs_to_nobody_once_accounts_exist() {
        // A NULL `user_id` must not silently match a signed-in viewer — Phase 5 backfills
        // those rows with a real account id rather than relying on a match here.
        assert!(!review(None).is_by(Some("me")));
    }
}
