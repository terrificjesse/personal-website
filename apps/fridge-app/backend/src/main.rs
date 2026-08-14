mod auth;
mod db;
mod expiration;
mod foodkeeper;
mod models;
mod nlp;
mod purchase_history;
mod recommend;
mod recommend_recipes;
mod rerank;
mod routes;
mod themealdb;

use std::net::SocketAddr;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Loads `apps/fridge-app/backend/.env` if present. Missing is fine — every value it
    // supplies has a working local-dev default, and Google OAuth simply stays unconfigured.
    // Real environment variables always win over the file.
    let _ = dotenvy::dotenv();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://fridge.db?mode=rwc".to_string());

    let pool = db::init_pool(&database_url).await?;

    match auth::purge_expired_sessions(&pool).await {
        Ok(0) => {}
        Ok(purged) => println!("purged {purged} expired sessions"),
        // Housekeeping only — `validate_session` rejects expired rows on its own, so failing
        // to tidy them is not a reason to refuse to start.
        Err(err) => eprintln!("could not purge expired sessions: {err:?}"),
    }

    let catalog = Arc::new(foodkeeper::Catalog::load()?);
    println!("loaded {} FoodKeeper names", catalog.entries().len());

    let recipe_catalog = Arc::new(themealdb::Catalog::load()?);
    println!("loaded {} TheMealDB recipes", recipe_catalog.recipes().len());

    let google_oauth = auth::GoogleOAuthConfig::from_env();
    match &google_oauth {
        Some(_) => println!("Google OAuth configured"),
        None => println!("Google OAuth not configured — password login only"),
    }

    let app = routes::build_router(routes::AppState {
        pool,
        catalog,
        recipe_catalog,
        google_oauth,
    });

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    println!("fridge_backend listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
