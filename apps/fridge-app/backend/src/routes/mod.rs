pub mod auth;
pub mod blog;
pub mod health;
pub mod items;
pub mod recipes;
pub mod reviews;
pub mod shopping_list;
pub mod suggest;

use std::sync::Arc;

use axum::{
    Router,
    extract::FromRef,
    http::{HeaderValue, Method, header},
    routing::{delete, get, patch, post},
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
    // App information for fetching from Google API, which is an optional sign-in option:
    pub google_oauth: Option<GoogleOAuthConfig>,
}

// Lets a handler obtain a cheap copy of the pool rather than the whole AppState
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

// Generates a Vector with the origins that the frontend is able to read responses from:
fn allowed_origins() -> Vec<HeaderValue> {
    let configured = std::env::var("ALLOWED_ORIGINS")
        .unwrap_or_else(|_| "http://localhost:3000,http://127.0.0.1:3000".to_string());

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

// This function first sets up the CORS layer for brower-enforced access control of communication between the frontend and backend.
// It then builds the router to match client HTTP requests to (path,method) pairs to yield the correct handler
pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(allowed_origins())
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::PATCH])
        .allow_headers([header::CONTENT_TYPE])
        // Enables the server to send a response to the client
        .allow_credentials(true);

    // Each route is a separate (path,method) pairing mapped to a handler:
    Router::<AppState>::new()
        .route("/health", get(health::health))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/google/start", get(auth::google_start))
        .route("/auth/google/callback", get(auth::google_callback))
        .route("/items", get(items::list_items).post(items::add_item))
        .route("/items/suggest", get(suggest::suggest_items))
        .route("/items/{id}", delete(items::remove_item))
        .route(
            "/shopping-list",
            get(shopping_list::list_shopping_list).post(shopping_list::add_shopping_list_item),
        )
        .route(
            "/shopping-list/suggestions",
            get(shopping_list::suggestions),
        )
        .route(
            "/shopping-list/{id}",
            delete(shopping_list::remove_shopping_list_item),
        )
        .route(
            "/shopping-list/{id}/purchase",
            post(shopping_list::mark_purchased),
        )
        .route("/recipes/recommended", get(recipes::recommended))
        .route("/recipes/liked", get(recipes::liked))
        .route(
            "/reviews",
            get(reviews::list_reviews).post(reviews::submit_review),
        )
        .route("/recipes/{id}/reviews", get(reviews::list_recipe_reviews))
        .route(
            "/blog/posts",
            get(blog::list_posts).post(blog::create_post),
        )
        .route("/blog/posts/by-slug/{slug}", get(blog::get_post))
        .route(
            "/blog/posts/{id}",
            patch(blog::update_post).delete(blog::delete_post),
        )
        .with_state(state)
        .layer(cors)
}
