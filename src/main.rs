use axum::{routing::get, Router};
use std::net::SocketAddr;
use tokio::net::TcpListener;

use optimo::routes;
use optimo::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    
// TEMP: routing is still defined here.
// This will move to a dedicated router module
// once application boundaries are finalized.

    let state = AppState::new().await;

    let app = Router::new()
        .route("/health", get(routes::health::health))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("Listening on {}", addr);

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}