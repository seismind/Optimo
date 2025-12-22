use axum::{routing::get, Router};
use std::net::SocketAddr;
use tokio::net::TcpListener;

mod routes;


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    
    // osservabilità / env (minimo vitale)
    
    tracing_subscriber::fmt::init();

    // router = traffico, non logica
    let app = Router::new()
        .route("/health", get(routes::health));


    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("Listening on {}", addr);

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

