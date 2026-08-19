use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct FridgeItem {
    // Fridge database row id
    pub id: String,
    pub canonical_name: String,
    pub quantity: f64,
    pub unit: String,
    pub added_at: DateTime<Utc>,
    pub estimated_expiration: Option<DateTime<Utc>>,
    /// The corresponding id in the FoodKeeper catalog. None if not present
    pub foodkeeper_product_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AddItemRequest {
    pub name: String,
    #[serde(default = "default_quantity")]
    pub quantity: f64,
    #[serde(default = "default_unit")]
    pub unit: String,
    /// The corresponding id in the FoodKeeper catalog. None if not present
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

/// Enum for labelling items currently in the shopping list
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
    /// Represents if the item is a grocery item so that it may be reflected in
    /// the fridge and purchase history
    pub is_grocery: bool,
    /// Tracks if the item was added manually or automatically suggest
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

/// Every time an item is added to the fridge it is reflected in purchase history
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PurchaseHistory {
    pub id: String,
    pub item_name: String,
    pub quantity: f64,
    pub purchased_at: DateTime<Utc>,
}

/// A record of the recipe ingredients, only the name is used for matching items
/// in the fridge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeIngredient {
    pub name: String,
    pub measure: String,
}

/// Struct representing a recipe
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub cuisine_tags: Vec<String>,
    pub meal_type_tags: Vec<String>,
    pub cook_time_minutes: Option<u32>,
    /// The appliances needed to cook the recipe
    pub required_appliances: Vec<String>,
    /// Curated ingredients that can plausibly be matched to fridge items
    pub fridge_ingredients: Vec<RecipeIngredient>,
    /// The nonfridge ingredients like salt or oil.
    pub extra_ingredients: Vec<RecipeIngredient>,
    pub image_url: Option<String>,
    /// The instructions for assembling the recipe
    pub instructions: String,
}

/// Struct representing a review
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Review {
    pub id: String,
    pub recipe_id: String,
    /// A rating on a scale of 1-5
    pub rating: i64,
    pub cooked_at: DateTime<Utc>,
    pub notes: Option<String>,
    /// The user that created the review.
    pub user_id: Option<String>,
    /// Represents if a review is visible to everyone or just the reviewer
    pub is_public: bool,
    /// Optional moderation control
    pub hidden: bool,
}

impl Review {
    /// Checks that the viewer of this review is the writer.
    pub fn is_by(&self, viewer: Option<&str>) -> bool {
        match viewer {
            None => true,
            Some(viewer_id) => self.user_id.as_deref() == Some(viewer_id),
        }
    }
}

/// Tracks user data
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: String,
    /// The email associated with the user
    pub email: String,
    /// The hashed password for security
    pub password_hash: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Documents the current session on the session table
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Session {
    pub id: String,
    /// The associated user with this session
    pub user_id: String,
    /// The hashed token for the session
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    /// When the current time exceeds this value, the user will be denied access
    /// until they sign into a new session
    pub expires_at: DateTime<Utc>,
}

// Tracks the identity of users signing in with external providers
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OAuthIdentity {
    pub id: String,
    /// Associated user
    pub user_id: String,
    /// Currently only Google
    pub provider: String,
    /// External provider associated id
    pub provider_account_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Threshold for labeling a recipe as liked
pub const LIKED_RATING_THRESHOLD: i64 = 4;

/// Threshold for excluding a certain recipe
pub const SUPPRESSED_RATING_THRESHOLD: i64 = 1;

pub const MIN_RATING: i64 = 1;
pub const MAX_RATING: i64 = 5;

/// 3.0
pub const NEUTRAL_RATING: f64 = (MIN_RATING + MAX_RATING) as f64 / 2.0;

/// The maximum size of a review
pub const MAX_NOTES_LENGTH: usize = 2_000;

#[derive(Debug, Deserialize)]
pub struct AddReviewRequest {
    pub recipe_id: String,
    pub rating: i64,
    #[serde(default)]
    pub notes: Option<String>,
    /// Documents when the review was written
    #[serde(default)]
    pub cooked_at: Option<DateTime<Utc>>,
    /// Documents if the review is supposed to be public or private
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
        assert!(!review(None).is_by(Some("me")));
    }
}
