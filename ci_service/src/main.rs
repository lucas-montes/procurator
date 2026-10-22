use std::path::PathBuf;

use axum::{routing::get, Router};
use ci_service::{routes, AppState, Database, JobQueue, Worker};
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::Config;

mod config;

/// Resolve `--config <path>`. Why: CLI arg only — no env pointer, no fixed path.
fn config_path() -> Result<PathBuf, String> {
    const USAGE: &str = "usage: ci_service --config <path.toml>";
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--config") => args
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| format!("{USAGE} (missing path after --config)")),
        Some(other) => Err(format!("{USAGE} (got {other:?})")),
        None => Err(USAGE.to_string()),
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Why: usage errors exit 2 with a one-line message, not a Debug dump.
    let path = match config_path() {
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
        Ok(p) => p,
    };
    // Fail-fast: unreadable, unparsable, or invalid bind/paths exits here.
    let config = Config::from_file(&path).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    info!(
        database = config.database_url.as_str(),
        bind_address = config.bind_address.as_str(),
        repos_path = config.repos_base_path.as_str(),
        path = %path.display(),
        "Starting Procurator CI Service"
    );

    // Initialize database
    let database = Database::new(&config.database_url).await.unwrap();

    // Initialize job queue and repository store
    let queue = JobQueue::new(database);

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
