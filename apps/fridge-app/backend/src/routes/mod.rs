pub mod auth;
pub mod health;
pub mod items;
pub mod recipes;
pub mod reviews;
pub mod shopping_list;
pub mod suggest;

use std::sync::Arc;

use axum::{
    extract::FromRef,
    http::{header, HeaderValue, Method},
    routing::{delete, get, post},
    Router,
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

use crate::auth::GoogleOAuthConfig;
use crate::foodkeeper::Catalog;
use crate::themealdb::Catalog as RecipeCatalog;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub catalog: Arc<Catalog>,
    pub recipe_catalog: Arc<RecipeCatalog>,
    /// `None` when the Google OAuth env vars aren't set. Password login works regardless —
    /// Google is the *alternate* method per PLAN.md, so an unconfigured client makes two
    /// routes return 501 rather than stopping the app from starting.
    pub google_oauth: Option<GoogleOAuthConfig>,
}

// These let handlers keep extracting just the piece of state they need
// (`State<SqlitePool>`, `State<Arc<Catalog>>`) instead of the whole struct. Written out
// rather than derived so we don't need axum's `macros` feature for two impls.
impl FromRef<AppState> for SqlitePool {
    fn from_ref(state: &AppState) -> Self {
        state.pool.clone()
    }
}

impl FromRef<AppState> for Arc<Catalog> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.catalog)
    }
}

impl FromRef<AppState> for Arc<RecipeCatalog> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.recipe_catalog)
    }
}

impl FromRef<AppState> for Option<GoogleOAuthConfig> {
    fn from_ref(state: &AppState) -> Self {
        state.google_oauth.clone()
    }
}

/// Origins allowed to make credentialed requests, from `ALLOWED_ORIGINS` (comma-separated).
///
/// **This cannot be a wildcard.** Through Phase 4 the CORS layer sent
/// `Access-Control-Allow-Origin: *`, which is fine for anonymous reads and is rejected
/// outright by browsers once a request carries credentials — the session cookie would simply
/// never be sent, with no error beyond a 401. Same applies to the method and header lists
/// below: `*` is not usable in a credentialed response, so both are enumerated.
///
/// The default covers local dev. Add the LAN origin (`http://192.168.x.x:3000`) to
/// `ALLOWED_ORIGINS` in `.env` when using the app from another device — see
/// `apps/fridge-app/CLAUDE.md` on the DHCP-assigned address.
fn allowed_origins() -> Vec<HeaderValue> {
    let configured = std::env::var("ALLOWED_ORIGINS").unwrap_or_else(|_| {
        "http://localhost:3000,http://127.0.0.1:3000".to_string()
    });

    configured
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .filter_map(|origin| match HeaderValue::from_str(origin) {
            Ok(value) => Some(value),
            Err(_) => {
                eprintln!("ignoring malformed origin in ALLOWED_ORIGINS: {origin:?}");
                None
            }
        })
        .collect()
}

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(allowed_origins())
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::PATCH])
        .allow_headers([header::CONTENT_TYPE])
        // Without this the browser strips the session cookie from cross-origin requests and
        // every call looks signed-out.
        .allow_credentials(true);

    // Annotated because the handlers extract sub-state (`State<SqlitePool>`), which would
    // otherwise make the router infer its state type as `SqlitePool` rather than `AppState`.
    //
    // Every route below except `/health` and `/auth/*` takes a `CurrentUser` extractor and is
    // therefore protected — there is no separate middleware list to keep in sync with this
    // table, and a route's own signature is the authority on whether it needs a session.
    Router::<AppState>::new()
        .route("/health", get(health::health))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/google/start", get(auth::google_start))
        .route("/auth/google/callback", get(auth::google_callback))
        .route("/items", get(items::list_items).post(items::add_item))
        // Static segments take priority over `{id}` in axum's router, so this is
        // unambiguous against the DELETE route below.
        .route("/items/suggest", get(suggest::suggest_items))
        .route("/items/{id}", delete(items::remove_item))
        .route(
            "/shopping-list",
            get(shopping_list::list_shopping_list).post(shopping_list::add_shopping_list_item),
        )
        // Static segment, same priority reasoning as `/items/suggest` above.
        .route("/shopping-list/suggestions", get(shopping_list::suggestions))
        .route("/shopping-list/{id}", delete(shopping_list::remove_shopping_list_item))
        .route("/shopping-list/{id}/purchase", post(shopping_list::mark_purchased))
        .route("/recipes/recommended", get(recipes::recommended))
        // Static segment, same priority reasoning as `/items/suggest` above.
        .route("/recipes/liked", get(recipes::liked))
        .route("/reviews", get(reviews::list_reviews).post(reviews::submit_review))
        // Public review wall for one recipe — the read half of the global aggregator.
        .route("/recipes/{id}/reviews", get(reviews::list_recipe_reviews))
        .with_state(state)
        .layer(cors)
}
