mod api;
mod queue;
mod worker;

use axum::{routing::post, Router};
use std::sync::Arc;
use tracing::info;

#[derive(Debug)]
enum Error {
    Database(String),
    Network(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Database(msg) => write!(f, "Database error: {}", msg),
            Error::Network(msg) => write!(f, "Network error: {}", msg),
        }
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub struct AppState {
    queue: Arc<queue::BuildQueue>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .init();

    info!("Starting Procurator CI Service");

    // Initialize database
    let queue = queue::BuildQueue::new("sqlite:ci.db")
        .await
        .map_err(|e| Error::Database(e.to_string()))?;

    let state = AppState {
        queue: Arc::new(queue),
    };

    // Start build worker in background
    let worker_state = state.clone();
    tokio::spawn(async move {
        let worker = worker::Worker::new(worker_state.queue.clone());
        if let Err(e) = worker.run().await {
            tracing::error!("Worker error: {}", e);
        }
    });

    // Build API
    let app = Router::new()
        .route("/api/builds", post(api::create_build))
        .route("/health", axum::routing::get(|| async { "OK" }))
        .with_state(state);

    // Start server
    let addr = "127.0.0.1:3000";
    info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| Error::Network(e.to_string()))?;

    axum::serve(listener, app)
        .await
        .map_err(|e| Error::Network(e.to_string()))?;

    Ok(())
}
