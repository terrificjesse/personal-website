pub mod health;
pub mod items;
pub mod suggest;

use std::sync::Arc;

use axum::{
    extract::FromRef,
    routing::{delete, get},
    Router,
};
use sqlx::SqlitePool;
use tower_http::cors::{Any, CorsLayer};

use crate::foodkeeper::Catalog;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub catalog: Arc<Catalog>,
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

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any);

    // Annotated because the handlers extract sub-state (`State<SqlitePool>`), which would
    // otherwise make the router infer its state type as `SqlitePool` rather than `AppState`.
    Router::<AppState>::new()
        .route("/health", get(health::health))
        .route("/items", get(items::list_items).post(items::add_item))
        // Static segments take priority over `{id}` in axum's router, so this is
        // unambiguous against the DELETE route below.
        .route("/items/suggest", get(suggest::suggest_items))
        .route("/items/{id}", delete(items::remove_item))
        .with_state(state)
        .layer(cors)
}
