mod auth;
mod blog_files;
mod db;
mod expiration;
mod foodkeeper;
mod hunt;
mod internships;
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

    // Reconcile the blog with the markdown files in content/blog/. Deliberately not fatal:
    // the blog falling back to database-only posts shouldn't stop the fridge app from serving.
    match blog_files::sync(&pool).await {
        Ok(blog_files::SyncOutcome::Completed(report))
            if report == blog_files::SyncReport::default() => {}
        Ok(blog_files::SyncOutcome::Completed(report)) => println!(
            "blog sync: {} created, {} updated, {} deleted, {} skipped",
            report.created, report.updated, report.deleted, report.skipped
        ),
        // Not a failure: the watcher will retry, so this is a state to report, not swallow.
        Ok(blog_files::SyncOutcome::Deferred(reason)) => {
            println!("blog sync: waiting — {reason}")
        }
        Err(err) => eprintln!("blog sync failed, serving database posts only: {err:?}"),
    }

    // Then keep watching, so dropping a .md file in doesn't need a restart or a button press.
    // Spawned after the startup sync above, which is what its first fingerprint is taken
    // against — otherwise it would immediately re-sync what startup just ingested.
    blog_files::spawn_watcher(pool.clone());

    // Internship collection + the expiry sweep. Both are cadenced by env vars and both
    // disable cleanly; see `internships::collector`. Spawned rather than awaited — a slow or
    // blocked job board must never delay the server binding its port.
    internships::collector::start(pool.clone()).await;

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

    // Sets up a socket listening connection address. The port is configurable so a second
    // instance (a throwaway database for verification, say) can run alongside the usual one
    // without either having to be stopped; the default is what everything else assumes.
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("fridge_backend listening on http://{addr}");

    // Awaits a connection from the frontend
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
