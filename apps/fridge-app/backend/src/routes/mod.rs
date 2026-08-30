pub mod auth;
pub mod blog;
pub mod health;
pub mod hunt;
pub mod internships;
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
use tower_http::cors::{AllowOrigin, CorsLayer};

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

/// The scheme every Firefox extension page is served from.
const FIREFOX_EXTENSION_SCHEME: &str = "moz-extension://";

/// Whether a request's `Origin` may read our responses.
///
/// Two ways in, and the second is a deliberate widening made on 2026-08-30:
///
/// 1. It is listed in `ALLOWED_ORIGINS` — the site itself.
/// 2. It is **any** `moz-extension://` origin — the hunt extension (Phase 8).
///
/// # Why the extension needs naming at all
///
/// The extension fetches with `credentials: "include"`, which makes every call a credentialed
/// cross-origin request, and the browser **discards the response** unless it carries an
/// `Access-Control-Allow-Origin` naming the caller. Reaching JS as a bare `TypeError`
/// indistinguishable from a dead server, this cost an evening: the extension reported "can't
/// reach localhost" while `curl` got 200s from the same URL.
///
/// # Why any extension rather than one
///
/// An extension's origin is `moz-extension://<uuid>` where the UUID is generated **per Firefox
/// profile**, so pinning it means a per-machine, per-profile `.env` edit that breaks on a new
/// profile and reads like a bug when it does.
///
/// The cost, accepted knowingly by the user: any Firefox extension they install could call
/// this API with their session cookie attached. That is narrower than it sounds — the
/// extension must also hold a host permission for this origin, which Firefox makes the user
/// grant per extension — but it is wider than naming one UUID, and it is the reason this is a
/// documented decision rather than a convenience.
///
/// **This is a local development posture.** Before deploying anywhere reachable from the
/// internet, revisit it alongside the other three items in `docs/PLAN.md` § After Phase 5
/// (`COOKIE_SECURE`, `SameSite`, rate limiting).
fn is_allowed_origin(configured: &[HeaderValue], origin: &HeaderValue) -> bool {
    if configured.iter().any(|allowed| allowed == origin) {
        return true;
    }
    origin
        .as_bytes()
        .starts_with(FIREFOX_EXTENSION_SCHEME.as_bytes())
}

// This function first sets up the CORS layer for brower-enforced access control of communication between the frontend and backend.
// It then builds the router to match client HTTP requests to (path,method) pairs to yield the correct handler
pub fn build_router(state: AppState) -> Router {
    let configured = allowed_origins();
    let cors = CorsLayer::new()
        // A predicate rather than a list, because the set is no longer enumerable in advance.
        // It still echoes the specific requesting origin rather than `*`, which is required
        // for a credentialed request and is why this cannot simply be `allow_origin(Any)`.
        .allow_origin(AllowOrigin::predicate(move |origin, _parts| {
            is_allowed_origin(&configured, origin)
        }))
        // PUT joins the list for `/hunt/profile`. A method missing here is refused at the
        // preflight, which surfaces as a failed fetch rather than as a 405 — the same
        // "looks like the backend is down" trap the AUTHORIZATION header hit.
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
        ])
        // AUTHORIZATION is here for the extension's bearer token. Without it the preflight
        // refuses the header and the request never arrives — which looks, once again, exactly
        // like a backend that is down.
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
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
        .route("/blog/posts", get(blog::list_posts).post(blog::create_post))
        .route("/blog/posts/by-slug/{slug}", get(blog::get_post))
        .route("/blog/sync", post(blog::sync_posts))
        .route(
            "/internships/applications",
            get(internships::list_applications).post(internships::create_application),
        )
        .route("/hunt/events", get(hunt::list_events))
        .route("/hunt/events/{id}/ack", post(hunt::ack_event))
        .route(
            "/hunt/tokens",
            get(hunt::list_tokens).post(hunt::create_token),
        )
        .route("/hunt/tokens/{id}", delete(hunt::revoke_token))
        .route(
            "/hunt/profile",
            get(hunt::get_profile).put(hunt::put_profile),
        )
        .route("/hunt/posting-for", get(hunt::posting_for_page))
        .route("/internships", get(internships::list_postings))
        .route("/internships/sources", get(internships::list_sources))
        .route("/internships/collect", post(internships::collect_now))
        .route("/internships/runs", get(internships::run_health))
        .route(
            "/internships/runs/{source_run_id}/rejects",
            get(internships::list_rejects),
        )
        .route(
            "/internships/applications/{id}",
            patch(internships::update_application).delete(internships::delete_application),
        )
        .route(
            "/blog/posts/{id}",
            patch(blog::update_post).delete(blog::delete_post),
        )
        .with_state(state)
        .layer(cors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured() -> Vec<HeaderValue> {
        vec![HeaderValue::from_static("http://localhost:3000")]
    }

    #[test]
    fn the_configured_site_origin_is_allowed() {
        assert!(is_allowed_origin(
            &configured(),
            &HeaderValue::from_static("http://localhost:3000")
        ));
    }

    #[test]
    fn any_firefox_extension_origin_is_allowed() {
        // Per-profile UUIDs, so the set cannot be enumerated in advance.
        for uuid in [
            "moz-extension://11111111-2222-3333-4444-555555555555",
            "moz-extension://deadbeef-0000-0000-0000-000000000000",
        ] {
            assert!(
                is_allowed_origin(&configured(), &HeaderValue::from_str(uuid).unwrap()),
                "{uuid} should be allowed"
            );
        }
    }

    #[test]
    fn an_unrelated_origin_is_still_refused() {
        for origin in [
            "https://evil.example.com",
            "http://localhost:3001",
            // The prefix check must be on the scheme, not a substring anywhere in the value.
            "https://moz-extension.evil.example.com",
            "moz-extension-evil://11111111-2222-3333-4444-555555555555",
        ] {
            assert!(
                !is_allowed_origin(&configured(), &HeaderValue::from_str(origin).unwrap()),
                "{origin} should be refused"
            );
        }
    }
}
