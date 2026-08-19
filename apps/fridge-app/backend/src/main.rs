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
    // Load the .env file:
    let _ = dotenvy::dotenv();

    // Load the .env database url or default otherwise
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://fridge.db?mode=rwc".to_string());

    let pool = db::init_pool(&database_url).await?;

    // Checks for expired sessions in the session table and purges them
    match auth::purge_expired_sessions(&pool).await {
        Ok(0) => {}
        Ok(purged) => println!("purged {purged} expired sessions"),
        Err(err) => eprintln!("could not purge expired sessions: {err:?}"),
    }

    // Load the FoodKeeper and MealDB catalogs
    let catalog = Arc::new(foodkeeper::Catalog::load()?);
    println!("loaded {} FoodKeeper names", catalog.entries().len());

    let recipe_catalog = Arc::new(themealdb::Catalog::load()?);
    println!(
        "loaded {} TheMealDB recipes",
        recipe_catalog.recipes().len()
    );

    // Sets up the Google OAuth authentication from the .env file
    let google_oauth = auth::GoogleOAuthConfig::from_env();
    match &google_oauth {
        Some(_) => println!("Google OAuth configured"),
        None => println!("Google OAuth not configured — password login only"),
    }

    // Sets up the router so the server is ready to handle HTTP requests from the frontend when the listening socket is set up
    let app = routes::build_router(routes::AppState {
        pool,
        catalog,
        recipe_catalog,
        google_oauth,
    });

    // Sets up a socket listening connection address
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    println!("fridge_backend listening on http://{addr}");

    // Awaits a connection from the frontend
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
