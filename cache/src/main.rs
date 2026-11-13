//! `cache` binary: the composition root.
//! Why: all wiring lives here — config in, adapters chosen, HTTP served.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cache::adapters::fs::FsBlobStore;
use cache::adapters::http::router;
use cache::adapters::sqlite::SqliteIndex;
use cache::adapters::system::SystemEnv;
use cache::application::Service;
use cache::application::ports::{BlobError, IndexError};
use cache::config::{Config, ConfigError};

/// Startup failures.
#[derive(Debug)]
enum MainError {
    Config(ConfigError),
    Blobs(BlobError),
    Index(IndexError),
    Bind(std::io::Error),
    Serve(std::io::Error),
}

impl fmt::Display for MainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(e) => e.fmt(f),
            Self::Blobs(e) => e.fmt(f),
            Self::Index(e) => e.fmt(f),
            Self::Bind(e) => write!(f, "bind: {e}"),
            Self::Serve(e) => write!(f, "serve: {e}"),
        }
    }
}

impl std::error::Error for MainError {}

/// Resolve `--config <path>`. Why: CLI arg only — no env pointer, no fixed path.
fn config_path() -> Result<PathBuf, String> {
    const USAGE: &str = "usage: cache --config <path.toml>";
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
    tracing::info!("shutting down");
}

/// Run GC on a fixed interval for as long as the process lives.
async fn gc_loop(service: Arc<Service<FsBlobStore, SqliteIndex, SystemEnv>>, every: Duration) {
    let mut ticker = tokio::time::interval(every);
    // Why: the first tick fires immediately, cleaning up after a restart.
    loop {
        ticker.tick().await;
        match service.collect_garbage().await {
            Ok(report) => tracing::info!(?report, "gc pass finished"),
            Err(error) => tracing::error!(%error, "gc pass failed"),
        }
    }
}

async fn run(config: Config) -> Result<(), MainError> {
    let addr = config.addr();
    let gc_interval = config.gc_interval();
    let orphan_grace_secs = config.orphan_grace_secs();
    let admin_token = config.admin_token();
    std::fs::create_dir_all(config.storage_dir()).map_err(MainError::Bind)?;
    let blobs = FsBlobStore::open(config.storage_dir()).map_err(MainError::Blobs)?;
    let index = SqliteIndex::open(&config.storage_dir().join("index.sqlite"))
        .await
        .map_err(MainError::Index)?;
    if admin_token.is_none() {
        tracing::warn!("no admin_token_sha256 configured: tokens can only be used, not created");
    }
    let service = Arc::new(Service::new(
        blobs,
        index,
        SystemEnv,
        config.into_caches(),
        admin_token,
        orphan_grace_secs,
    ));
    for spec in service.caches() {
        tracing::info!(
            cache = spec.name().as_str(),
            visibility = ?spec.visibility(),
            public_key = spec.signing_key().public_key(),
            "serving cache"
        );
    }
    let gc = tokio::spawn(gc_loop(Arc::clone(&service), gc_interval));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(MainError::Bind)?;
    tracing::info!(%addr, "cache listening (plain HTTP; terminate TLS in front)");
    let served = axum::serve(listener, router(service))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(MainError::Serve);
    gc.abort();
    served
}

#[tokio::main]
async fn main() -> Result<(), MainError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();
    let path = match config_path() {
        Ok(path) => path,
        // Why: usage errors exit 2 with one line, not a Debug dump.
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    run(Config::from_file(&path).map_err(MainError::Config)?).await
}
