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
    /// Grants access to admin-only features (currently: the blog editor). Not settable
    /// through any API — see migration `0009_add_user_admin_flag.sql`.
    pub is_admin: bool,
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

/// A blog post. Every post has an author and is either a draft (visible only to admins) or
/// published (visible to everyone) — see `routes/blog.rs`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BlogPost {
    pub id: String,
    pub author_id: String,
    pub title: String,
    /// URL-safe identifier, derived from the title at creation time and stable afterwards.
    pub slug: String,
    pub body: String,
    pub published: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// `"db"` for a post written through `/blog/admin`, `"file"` for one ingested from
    /// `content/blog/*.md` by `blog_files::sync`. Both kinds live in the same table so that
    /// sort and search have one query path; see migration `0011`.
    pub source: String,
}

/// The value of `BlogPost::source` for a post written in the browser.
pub const BLOG_SOURCE_DB: &str = "db";
/// The value of `BlogPost::source` for a post ingested from a markdown file.
pub const BLOG_SOURCE_FILE: &str = "file";

/// Ordering for `GET /blog/posts`. Named rather than a raw `&str` so the only two strings
/// the API accepts are pinned in one place, and so an unrecognized value is rejected by the
/// `Query` extractor as a 400 instead of silently falling back to a default — a silent
/// fallback would turn `?sort=oldset` into "newest" with no way to notice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[default]
    Newest,
    Oldest,
}

impl SortOrder {
    /// The SQL direction this maps to. Returns a `&'static str` rather than anything
    /// caller-supplied, because it is interpolated into the statement — `ORDER BY` cannot
    /// take a bind parameter, so this is the one part of the query that must not be
    /// reachable from user input.
    pub fn sql_direction(self) -> &'static str {
        match self {
            SortOrder::Newest => "DESC",
            SortOrder::Oldest => "ASC",
        }
    }
}

/// Query string for `GET /blog/posts`. Both fields are optional; absent `sort` means newest
/// first, which is what the endpoint did before either existed.
#[derive(Debug, Default, Deserialize)]
pub struct ListPostsQuery {
    #[serde(default)]
    pub sort: SortOrder,
    /// Free-text search across title and body. Whitespace-only is treated as absent.
    pub q: Option<String>,
    /// How many posts to return. Absent means `DEFAULT_BLOG_PAGE_SIZE`.
    ///
    /// `u32` rather than `i64` so a negative value is rejected by the `Query` extractor as a
    /// 400 before it can reach SQL, where `LIMIT -1` means *no limit*.
    pub limit: Option<u32>,
    /// How many to skip. Absent means 0.
    pub offset: Option<u32>,
}

/// Returned when no `limit` is given.
pub const DEFAULT_BLOG_PAGE_SIZE: u32 = 20;

/// The largest page a caller may ask for.
///
/// Asking for more is a **400, not a silent clamp**. A clamp would hand back 100 posts to a
/// caller who asked for 1000 and believes it now holds everything — the same
/// looks-complete-but-isn't failure that makes `?sort=oldset` a 400 rather than a quiet
/// fallback to newest.
pub const MAX_BLOG_PAGE_SIZE: u32 = 100;

/// A default larger than the maximum would make every unparameterised request a 400. Checked
/// at compile time rather than in a test, so it cannot be broken by an edit that skips `cargo
/// test`.
const _: () = assert!(DEFAULT_BLOG_PAGE_SIZE <= MAX_BLOG_PAGE_SIZE);

/// One page of posts.
///
/// An envelope rather than a bare array because the count is not derivable from the page: a
/// full page tells you nothing about whether more exist, and the admin view needs to say how
/// many posts there are. Sending it in a header instead would need
/// `Access-Control-Expose-Headers`, and a header the browser silently refuses to expose is a
/// worse failure than a slightly larger body.
#[derive(Debug, Clone, Serialize)]
pub struct BlogPostPage {
    pub posts: Vec<BlogPost>,
    /// Total matching posts **the requester may see** — filtered exactly like the page, so it
    /// never reveals how many drafts exist to someone who cannot read them.
    pub total: i64,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Deserialize)]
pub struct CreateBlogPostRequest {
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub published: bool,
}

/// Every field optional: only what's present in the request changes.
#[derive(Debug, Deserialize)]
pub struct UpdateBlogPostRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub published: Option<bool>,
}

pub const MAX_BLOG_TITLE_LENGTH: usize = 200;
pub const MAX_BLOG_BODY_LENGTH: usize = 100_000;

/// Whether `text` is longer than `max` **characters**.
///
/// Exists because `str::len()` is a byte count, and using it here silently made the limits
/// stricter for anyone not writing in ASCII: a 200-character CJK title is 600 bytes and was
/// rejected, despite `docs/BLOG.md` documenting the limit in characters. The limits are a
/// contract with the author about how much they may write, so characters is the honest unit.
///
/// `chars()` counts Unicode scalar values, not grapheme clusters — a family emoji or a
/// combining accent still costs more than one. That is a far smaller discrepancy than
/// bytes-vs-chars, and closing it would mean a dependency for a limit nobody writes up
/// against.
pub fn exceeds_char_limit(text: &str, max: usize) -> bool {
    text.chars().count() > max
}

/// Whether `text` is empty or contains nothing but whitespace.
///
/// Named and shared because the two ingest paths disagreed: `create_post` tested
/// `is_empty()`, so a body of `"   "` was a valid post through the API, while the file path
/// tested `trim().is_empty()` and rejected the same content. The same post was simultaneously
/// legal and illegal depending on how it arrived.
///
/// Only ever used to *validate*. The body is stored exactly as written — trimming it here
/// would silently rewrite an author's markdown, and leading whitespace is significant to an
/// indented code block.
pub fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

/// Turns a title into a URL-safe slug: lowercased, runs of non-alphanumeric characters
/// collapsed to a single hyphen, no leading or trailing hyphen. Not unique by itself — see
/// `routes/blog.rs::unique_slug` for the collision handling a plain function can't do.
pub fn slugify(title: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut last_was_hyphen = true; // suppresses a leading hyphen

    for ch in title.chars() {
        if ch.is_alphanumeric() {
            slug.extend(ch.to_lowercase());
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            slug.push('-');
            last_was_hyphen = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    slug
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

    #[test]
    fn slugify_lowercases_and_hyphenates() {
        assert_eq!(slugify("Hello World"), "hello-world");
    }

    #[test]
    fn slugify_collapses_punctuation_runs_to_one_hyphen() {
        assert_eq!(slugify("A, B & C -- Ready?"), "a-b-c-ready");
    }

    #[test]
    fn slugify_trims_leading_and_trailing_punctuation() {
        assert_eq!(slugify("  --Fridge Notes!!--  "), "fridge-notes");
    }

    #[test]
    fn slugify_of_an_all_punctuation_title_is_empty() {
        assert_eq!(slugify("!!!"), "");
    }

    // `Query` deserializes with `serde_urlencoded`, which isn't a direct dependency here.
    // These go through `serde_json` instead: the behavior under test is the *derived*
    // `Deserialize` impl — the `#[default]` and the rejection of an unknown unit variant —
    // and that is decided by the derive, not by the wire format.
    fn parse_sort(value: &str) -> Result<SortOrder, serde_json::Error> {
        serde_json::from_value(serde_json::Value::String(value.to_string()))
    }

    #[test]
    fn blankness_covers_every_flavour_of_whitespace() {
        assert!(is_blank(""));
        assert!(is_blank("   "));
        assert!(is_blank("\t\n\r\n  "));
        assert!(is_blank("\u{00a0}"), "a non-breaking space is still blank");

        assert!(!is_blank("a"));
        assert!(!is_blank("  a  "));
    }

    #[test]
    fn limits_are_measured_in_characters_not_bytes() {
        let ascii = "a".repeat(MAX_BLOG_TITLE_LENGTH);
        let cjk = "字".repeat(MAX_BLOG_TITLE_LENGTH);

        assert!(
            cjk.len() > MAX_BLOG_TITLE_LENGTH,
            "premise: bytes exceed chars"
        );
        assert!(!exceeds_char_limit(&ascii, MAX_BLOG_TITLE_LENGTH));
        assert!(
            !exceeds_char_limit(&cjk, MAX_BLOG_TITLE_LENGTH),
            "a non-ASCII title at exactly the limit is within it"
        );

        assert!(exceeds_char_limit(
            &"a".repeat(MAX_BLOG_TITLE_LENGTH + 1),
            MAX_BLOG_TITLE_LENGTH
        ));
        assert!(exceeds_char_limit(
            &"字".repeat(MAX_BLOG_TITLE_LENGTH + 1),
            MAX_BLOG_TITLE_LENGTH
        ));
    }

    #[test]
    fn paging_params_default_and_are_bounded() {
        let q: ListPostsQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(q.limit, None, "absent means the handler's default");
        assert_eq!(q.offset, None);

        let q: ListPostsQuery = serde_json::from_str(r#"{"limit":50,"offset":100}"#).unwrap();
        assert_eq!((q.limit, q.offset), (Some(50), Some(100)));

        // `u32`, so a negative never reaches SQL — where `LIMIT -1` means *no limit*, which
        // would turn a typo into "return everything" rather than an error.
        assert!(serde_json::from_str::<ListPostsQuery>(r#"{"limit":-1}"#).is_err());
        assert!(serde_json::from_str::<ListPostsQuery>(r#"{"offset":-1}"#).is_err());
        assert!(serde_json::from_str::<ListPostsQuery>(r#"{"limit":"20"}"#).is_err());
    }

    #[test]
    fn absent_sort_means_newest_first() {
        let query: ListPostsQuery =
            serde_json::from_str("{}").expect("both fields are optional, so {} is valid");
        assert_eq!(query.sort, SortOrder::Newest);
        assert_eq!(query.sort.sql_direction(), "DESC");
        assert!(query.q.is_none());
    }

    #[test]
    fn sort_accepts_exactly_newest_and_oldest() {
        assert_eq!(parse_sort("newest").unwrap(), SortOrder::Newest);
        assert_eq!(parse_sort("oldest").unwrap(), SortOrder::Oldest);
        assert_eq!(parse_sort("oldest").unwrap().sql_direction(), "ASC");
    }

    /// The point of the enum: a typo has to be an error rather than a silent "newest". This
    /// is what the `Query` extractor turns into a 400.
    #[test]
    fn an_unrecognized_sort_is_rejected_rather_than_defaulted() {
        assert!(parse_sort("oldset").is_err());
        assert!(parse_sort("").is_err());
        assert!(parse_sort("DESC").is_err());
    }
}
