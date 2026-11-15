//! `ci_service` binary: the composition root for a standalone deployment.
//! Why: all wiring lives here — config in, adapters chosen, HTTP served.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ci_service::adapters::http::router;
use ci_service::adapters::nix::{GitConfigSource, NixBuilder};
use ci_service::adapters::sqlite::SqliteRunStore;
use ci_service::adapters::system::SystemClock;
use ci_service::application::ports::StoreError;
use ci_service::application::{Service, ServiceError};
use ci_service::config::{Config, ConfigError};
use ci_service::domain::repo_config::{CachePolicy, Limits};
use tokio::task::JoinSet;
use tracing_subscriber::EnvFilter;

/// Startup failures.
#[derive(Debug)]
enum MainError {
    Config(ConfigError),
    Store(StoreError),
    Recover(ServiceError),
    Bind(std::io::Error),
    Serve(std::io::Error),
}

impl fmt::Display for MainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(e) => e.fmt(f),
            Self::Store(e) => e.fmt(f),
            Self::Recover(e) => write!(f, "recover interrupted runs: {e}"),
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
    let store = SqliteRunStore::open(&config.database_url)
        .await
        .map_err(MainError::Store)?;
    let repos = PathBuf::from(&config.repos_base_path);
    let mut builder = NixBuilder::new(repos.clone(), config.eval_workers, &config.allowed_uris);
    let mut cache = None;
    if let Some(base) = &config.cache_base_url {
        builder = builder.with_cache(base, config.netrc_file.as_ref().map(PathBuf::from));
        cache = Some(CachePolicy::new(
            config.public_cache.clone(),
            config.private_cache_prefix.clone(),
        ));
    }
    let limits = Limits::new(
        config.max_reruns,
        config.max_check_timeout,
        config.max_run_timeout,
    );
    let service = Arc::new(Service::new(
        store,
        SystemClock,
        builder,
        GitConfigSource::new(repos),
        limits,
        cache,
    ));

    // Why: no worker is running yet, so every `running` row is a leftover.
    let recovered = service.recover().await.map_err(MainError::Recover)?;
    if recovered > 0 {
        tracing::warn!(
            recovered,
            "requeued runs interrupted by the previous process"
        );
    }

    let listener = tokio::net::TcpListener::bind(&config.bind_address)
        .await
        .map_err(MainError::Bind)?;
    tracing::info!(
        bind_address = config.bind_address.as_str(),
        workers = config.max_concurrent_runs,
        "serving CI API"
    );

    // Why: dropping the set on return aborts the workers; a run cut off
    // this way is requeued at the next start.
    let mut workers = JoinSet::new();
    let poll = Duration::from_millis(config.worker_poll_interval_ms);
    for _ in 0..config.max_concurrent_runs {
        let service = Arc::clone(&service);
        workers.spawn(async move { service.work(poll).await });
    }
    axum::serve(listener, router(service))
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
