use axum::{routing::get, Router};
use ci_service::{routes, AppState, Config, Database, JobQueue, Worker};
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::default();

    info!(
        database = config.database_url.as_str(),
        bind_address = config.bind_address.as_str(),
        repos_path = config.repos_base_path.as_str(),
        "Starting Procurator CI Service"
    );

    // Initialize database
    let database = Database::new(&config.database_url).await.unwrap();

    // Initialize job queue and repository store
    let queue = Arc::new(JobQueue::new(database));

    let worker = Worker::new(queue.clone());
    let state = AppState::new(queue);

    tokio::spawn(worker.run());

    info!(target: "ciService", "Build worker spawned");

    let app = Router::new()
        .nest("/api", routes())
        .route("/health", get(|| async { "OK" }))
        .with_state(state);

    info!(target: "ciService", bind_address = config.bind_address.as_str(), "Starting HTTP server");

    let listener = tokio::net::TcpListener::bind(&config.bind_address)
        .await
        .unwrap();

    axum::serve(listener, app)
        .with_graceful_shutdown(signal())
        .await
        .unwrap();
}

async fn signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C signal handler");
    info!(target: "ciService", "Shutdown signal received, terminating...");
}
