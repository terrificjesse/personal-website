pub mod health;
pub mod items;

use axum::{
    routing::{delete, get},
    Router,
};
use sqlx::SqlitePool;
use tower_http::cors::{Any, CorsLayer};

pub fn build_router(pool: SqlitePool) -> Router {
    let cors = CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any);

    Router::new()
        .route("/health", get(health::health))
        .route("/items", get(items::list_items).post(items::add_item))
        .route("/items/{id}", delete(items::remove_item))
        .with_state(pool)
        .layer(cors)
}
