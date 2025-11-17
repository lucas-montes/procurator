mod api;
mod config;
mod error;
mod git_url;
mod queue;
mod repo_manager;
mod web;
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
    repo_manager: Arc<repo_manager::RepoManager>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .init();

    // Initialize config singleton
    let config = config::Config::init();

    info!("Starting Procurator CI Service");
    info!("Database: {}", config.database_url);
    info!("Repos base path: {}", config.repos_base_path);
    info!("Bind address: {}", config.bind_address);

    // Initialize database
    let queue = queue::BuildQueue::new(&config.database_url)
        .await
        .map_err(|e| Error::Database(e.to_string()))?;

    // Initialize repository manager (post-receive hook is now embedded)
    let repo_manager = repo_manager::RepoManager::new(&config.repos_base_path);

    let state = AppState {
        queue: Arc::new(queue),
        repo_manager: Arc::new(repo_manager),
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
        // Web UI
        .route("/", axum::routing::get(web::index))
        // API - Builds
        .route("/api/builds", post(api::create_build))
        .route("/api/builds", axum::routing::get(web::list_builds))
        .route("/api/builds/:id", axum::routing::get(web::get_build))
        .route("/api/builds/:id/logs", axum::routing::get(web::get_build_logs))
        // API - Repos
        .route("/api/repos", axum::routing::get(web::list_repos))
        .route("/api/repos", post(web::create_repo))
        .route("/api/repos/:name", axum::routing::get(web::get_repo))
        // Real-time events
        .route("/api/events", axum::routing::get(web::build_events))
        // Health check
        .route("/health", axum::routing::get(|| async { "OK" }))
        .with_state(state);

    // Start server
    info!("Listening on {}", config.bind_address);

    let listener = tokio::net::TcpListener::bind(&config.bind_address)
        .await
        .map_err(|e| Error::Network(e.to_string()))?;

    axum::serve(listener, app)
        .await
        .map_err(|e| Error::Network(e.to_string()))?;

    Ok(())
}
