//! `procurator`: the monolith's composition root. Runs the CI service,
//! repohub, the cache and optionally the control plane in one process, each
//! on the address its own config names.
//! Why: one unit to deploy on a homelab. Crates call each other's
//! application services directly here, never over HTTP; only external
//! programs (the git hook, Nix, workers) use the network APIs.

// Why: project rule — async functions return `impl Future + Send` explicitly.
#![allow(clippy::manual_async_fn)]

mod config;
mod deploy;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use cache::adapters::fs::FsBlobStore;
use cache::adapters::sqlite::SqliteIndex;
use cache::adapters::system::SystemEnv;
use ci_service::adapters::access::OpenAccess;
use ci_service::adapters::webhook::HttpDeploy;
use ci_service::runtime::{Runtime, StartError};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use self::config::{Config, ConfigError};
use self::deploy::ControlPlaneDeploy;

/// Startup and serving failures, by service.
#[derive(Debug)]
enum MainError {
    Config(ConfigError),
    CiConfig(ci_service::config::ConfigError),
    CiStart(StartError),
    ControlPlane(String),
    RepohubConfig(repohub::config::ConfigError),
    RepohubDatabase(repohub::adapters::shared::database::DatabaseError),
    CacheConfig(cache::config::ConfigError),
    CacheBlobs(cache::application::ports::BlobError),
    CacheIndex(cache::application::ports::IndexError),
    Bind(&'static str, std::io::Error),
    Serve(&'static str, std::io::Error),
}

impl fmt::Display for MainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(e) => e.fmt(f),
            Self::CiConfig(e) => write!(f, "ci: {e}"),
            Self::CiStart(e) => write!(f, "ci: {e}"),
            Self::ControlPlane(e) => write!(f, "control plane: {e}"),
            Self::RepohubConfig(e) => write!(f, "repohub: {e}"),
            Self::RepohubDatabase(e) => write!(f, "repohub: {e}"),
            Self::CacheConfig(e) => write!(f, "cache: {e}"),
            Self::CacheBlobs(e) => write!(f, "cache: {e}"),
            Self::CacheIndex(e) => write!(f, "cache: {e}"),
            Self::Bind(service, e) => write!(f, "{service}: bind: {e}"),
            Self::Serve(service, e) => write!(f, "{service}: serve: {e}"),
        }
    }
}

impl std::error::Error for MainError {}

/// A started service: its router and where to serve it.
struct Started {
    name: &'static str,
    router: Router,
    address: String,
}

/// Resolve `--config <path>`. Why: CLI arg only — no env pointer, no fixed path.
fn config_path() -> Result<PathBuf, String> {
    const USAGE: &str = "usage: procurator --config <path.toml>";
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

/// Run cache GC on a fixed interval for as long as the process lives.
async fn gc_loop(
    service: Arc<cache::application::Service<FsBlobStore, SqliteIndex, SystemEnv>>,
    every: Duration,
) {
    let mut ticker = tokio::time::interval(every);
    loop {
        ticker.tick().await;
        match service.collect_garbage().await {
            Ok(report) => tracing::info!(?report, "cache gc pass finished"),
            Err(error) => tracing::error!(%error, "cache gc pass failed"),
        }
    }
}

/// Open the cache's storage and start its GC.
async fn start_cache(path: &std::path::Path) -> Result<(Started, JoinHandle<()>), MainError> {
    let config = cache::config::Config::from_file(path).map_err(MainError::CacheConfig)?;
    let address = config.addr().to_string();
    let gc_interval = config.gc_interval();
    let orphan_grace_secs = config.orphan_grace_secs();
    let admin_token = config.admin_token();
    std::fs::create_dir_all(config.storage_dir()).map_err(|e| MainError::Bind("cache", e))?;
    let blobs = FsBlobStore::open(config.storage_dir()).map_err(MainError::CacheBlobs)?;
    let index = SqliteIndex::open(&config.storage_dir().join("index.sqlite"))
        .await
        .map_err(MainError::CacheIndex)?;
    if admin_token.is_none() {
        tracing::warn!(
            "cache: no admin_token_sha256 configured: tokens can only be used, not created"
        );
    }
    let service = Arc::new(cache::application::Service::new(
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
    let router = cache::adapters::http::router(service);
    Ok((
        Started {
            name: "cache",
            router,
            address,
        },
        gc,
    ))
}

/// Open repohub's database and build its web UI.
async fn start_repohub(path: &std::path::Path) -> Result<Started, MainError> {
    let config = repohub::Config::from_file(path).map_err(MainError::RepohubConfig)?;
    let db = repohub::Database::new(&config.database_url)
        .await
        .map_err(MainError::RepohubDatabase)?;
    let github = repohub::GithubAppState::new(db.clone(), &config);
    let gerrit = repohub::GerritAppState::new(db);
    let router = repohub::github_routes()
        .with_state(github)
        .merge(Router::new().nest("/gerrit", repohub::gerrit_routes().with_state(gerrit)));
    Ok(Started {
        name: "repohub",
        router,
        address: config.bind_address,
    })
}

/// Start the CI service. With a control plane config, the control plane
/// runs in this process and deploys go to it directly; otherwise they are
/// sent to each repo's `deploy.url` over HTTP.
async fn start_ci(
    config: ci_service::config::Config,
    control_plane: Option<&std::path::Path>,
) -> Result<Runtime, MainError> {
    let Some(path) = control_plane else {
        let deploys = HttpDeploy::new().map_err(|e| MainError::CiStart(StartError::Notifier(e)))?;
        return Runtime::start(config, OpenAccess, deploys)
            .await
            .map_err(MainError::CiStart);
    };
    let plane = control_plane::config::Config::read(path).map_err(MainError::ControlPlane)?;
    let publisher =
        control_plane::start(&plane).map_err(|e| MainError::ControlPlane(e.to_string()))?;
    Runtime::start(config, OpenAccess, ControlPlaneDeploy::new(publisher))
        .await
        .map_err(MainError::CiStart)
}

/// Serve one service until `stop` fires.
async fn serve(started: Started, mut stop: watch::Receiver<bool>) -> Result<(), MainError> {
    let listener = tokio::net::TcpListener::bind(&started.address)
        .await
        .map_err(|e| MainError::Bind(started.name, e))?;
    tracing::info!(
        service = started.name,
        address = started.address.as_str(),
        "listening"
    );
    axum::serve(listener, started.router)
        .with_graceful_shutdown(async move {
            // Why: a closed channel also means stop.
            let _ = stop.wait_for(|stopped| *stopped).await;
        })
        .await
        .map_err(|e| MainError::Serve(started.name, e))
}

async fn run(config: Config) -> Result<(), MainError> {
    let ci_config =
        ci_service::config::Config::from_file(config.ci()).map_err(MainError::CiConfig)?;
    // Why: repohub has no users, credentials or permissions yet, so it
    // cannot say who may read a repo; once it can, an adapter here wraps
    // its service and replaces this.
    // Why: held to the end; dropping it stops the CI workers.
    let runtime = start_ci(ci_config, config.control_plane()).await?;
    let repohub = start_repohub(config.repohub()).await?;
    let (cache, gc) = start_cache(config.cache()).await?;
    tracing::warn!("ci: open access: every caller reads every repo's runs and logs");

    let (stop, stopped) = watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = stop.send(true);
    });
    let ci = Started {
        name: "ci",
        router: runtime.router(),
        address: String::from(runtime.bind_address()),
    };
    let served = tokio::try_join!(
        serve(ci, stopped.clone()),
        serve(repohub, stopped.clone()),
        serve(cache, stopped),
    );
    gc.abort();
    drop(runtime);
    served.map(|_| ())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .log_internal_errors(true)
                .with_target(false)
                .flatten_event(true)
                .with_span_list(false),
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
        eprintln!("procurator: {e}");
        std::process::exit(1);
    }
}
