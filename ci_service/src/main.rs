//! `ci_service` binary: the composition root for a standalone deployment.
//! Why: all wiring lives in [`Runtime`]; this picks the access adapter and
//! serves HTTP.

use std::fmt;
use std::path::PathBuf;

use ci_service::adapters::access::OpenAccess;
use ci_service::config::{Config, ConfigError};
use ci_service::runtime::{Runtime, StartError};
use tracing_subscriber::EnvFilter;

/// Startup failures.
#[derive(Debug)]
enum MainError {
    Config(ConfigError),
    Start(StartError),
    Bind(std::io::Error),
    Serve(std::io::Error),
}

impl fmt::Display for MainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(e) => e.fmt(f),
            Self::Start(e) => e.fmt(f),
            Self::Bind(e) => write!(f, "bind: {e}"),
            Self::Serve(e) => write!(f, "serve: {e}"),
        }
    }
}

impl std::error::Error for MainError {}

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
        None => Err(String::from(USAGE)),
    }
}

/// Resolve when the process is asked to stop.
async fn shutdown_signal() {
    let interrupt = tokio::signal::ctrl_c();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    tokio::select! {
        _ = interrupt => {}
        _ = terminate.recv() => {}
    }
    tracing::info!("shutdown signal received");
}

async fn run(config: Config) -> Result<(), MainError> {
    // Why: repohub has no users or permissions yet, so a standalone CI
    // cannot tell readers apart either.
    let runtime = Runtime::start(config, OpenAccess)
        .await
        .map_err(MainError::Start)?;
    let listener = tokio::net::TcpListener::bind(runtime.bind_address())
        .await
        .map_err(MainError::Bind)?;
    tracing::info!(
        bind_address = runtime.bind_address(),
        "serving CI API (open access: every caller reads every repo)"
    );
    axum::serve(listener, runtime.router())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(MainError::Serve)
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
        Ok(path) => path,
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    let result = match Config::from_file(&path) {
        Ok(config) => run(config).await,
        Err(e) => Err(MainError::Config(e)),
    };
    if let Err(e) = result {
        eprintln!("ci_service: {e}");
        std::process::exit(1);
    }
}
