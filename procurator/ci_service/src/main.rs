mod api;
mod config;
mod database;
mod job_queue;
mod worker;

use axum::{routing::get, Router};
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Initialize config singleton
    let config = config::Config::init();

    info!(
        database = config.database_url.as_str(),
        bind_address = config.bind_address.as_str(),
        repos_path = config.repos_base_path.as_str(),
        "Starting Procurator CI Service"
    );

    // Initialize database
    let database = database::Database::new(&config.database_url)
        .await
        .map_err(|e| Error::Database(e.to_string()))?;

    // Initialize job queue and repository store
    let queue = Arc::new(job_queue::JobQueue::new(database.clone()));

    let worker = Worker::new(queue.clone());
    let state = AppState { queue };

    tokio::spawn(worker.run());

    info!(target: "ciService", "Build worker spawned");

    // Build API by merging routes from different modules
    let app = Router::new()
        .merge(web::routes())
        .nest("/api", api::routes())
        // Health check
        .route("/health", get(|| async { "OK" }))
        .with_state(state);

    // Start server
    info!(target: "ciService", bind_address = config.bind_address.as_str(), "Starting HTTP server");

    let listener = tokio::net::TcpListener::bind(&config.bind_address)
        .await
        .map_err(|e| Error::Network(e.to_string()))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(signal)
        .await
        .map_err(|e| Error::Network(e.to_string()))?;

    Ok(())
}

async fn signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C signal handler");
    info!(target: "ciService", "Shutdown signal received, terminating...");
}
