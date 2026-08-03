mod db;
mod expiration;
mod models;
mod nlp;
mod routes;

use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://fridge.db?mode=rwc".to_string());

    let pool = db::init_pool(&database_url).await?;
    let app = routes::build_router(pool);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    println!("fridge_backend listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
