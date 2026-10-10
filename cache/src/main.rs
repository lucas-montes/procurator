//! Procurator Nix binary cache: full read/write with mTLS.
//! Why: push-first flow needs `nix copy --to/--from https://cache` to just work.

mod config;
mod nix;
mod routes;
mod storage;
mod tls;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use config::Config;
use routes::{AppState, router};
use storage::LocalDisk;
use tls::{FilePki, TlsProvider as _, identity_from_leaf};

/// Concrete startup errors. Why: no `anyhow`, no `Box<dyn>`.
#[derive(Debug)]
enum MainError {
    /// CLI usage error (prints usage, exit code 2).
    Usage(String),
    /// Config invalid.
    Config(config::ConfigError),
    /// Storage init failed.
    Storage(storage::StorageError),
    /// TLS init failed.
    Tls(tls::TlsError),
    /// Listener bind failed.
    Bind(String),
    /// Serving failed.
    Serve(String),
}

impl std::fmt::Display for MainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(m) => write!(f, "{m}"),
            Self::Config(e) => write!(f, "config: {e}"),
            Self::Storage(e) => write!(f, "storage: {e}"),
            Self::Tls(e) => write!(f, "tls: {e}"),
            Self::Bind(m) => write!(f, "bind: {m}"),
            Self::Serve(m) => write!(f, "serve: {m}"),
        }
    }
}

impl std::error::Error for MainError {}

/// Resolve `--config <path>`. Why: CLI arg only — no env pointer, no fixed path.
fn config_path() -> Result<PathBuf, MainError> {
    const USAGE: &str = "usage: cache --config <path.toml>";
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--config") => args
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| MainError::Usage(format!("{USAGE} (missing path after --config)"))),
        Some(other) => Err(MainError::Usage(format!("{USAGE} (got {other:?})"))),
        None => Err(MainError::Usage(USAGE.to_string())),
    }
}

#[tokio::main]
async fn main() -> Result<(), MainError> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();
    let path = match config_path() {
        // Why: usage errors exit 2 with a one-line message, not a Debug dump.
        Err(MainError::Usage(m)) => {
            eprintln!("{m}");
            std::process::exit(2);
        }
        Err(e) => return Err(e),
        Ok(p) => p,
    };
    let config = Config::from_file(&path).map_err(MainError::Config)?;
    tracing::info!(
        addr = %config.addr,
        storage = %config.storage_dir.display(),
        mtls = config.mtls_enabled(),
        "starting procurator cache"
    );
    // Why: static dispatch — match picks a concrete backend, no `dyn`.
    let provider = config.tls.provider.clone();
    if provider != "file" {
        return Err(MainError::Tls(tls::TlsError::UnknownProvider(provider)));
    }
    let store = LocalDisk::create(config.storage_dir.clone(), config.max_size_bytes)
        .map_err(MainError::Storage)?;
    let state = AppState {
        config: Arc::new(config),
        store,
    };
    serve(state).await
}

/// Serve plain HTTP or HTTPS+mTLS depending on config.
async fn serve(state: AppState<LocalDisk>) -> Result<(), MainError> {
    let addr: SocketAddr = state
        .config
        .addr
        .parse()
        .map_err(|e| MainError::Bind(format!("bad addr: {e}")))?;
    let app = router(state.clone());
    if !state.config.tls_enabled() {
        tracing::warn!("TLS disabled — dev only, use mTLS in production");
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| MainError::Bind(e.to_string()))?;
        // Why: ConnectInfo feeds the loopback-bypass middleware (no TLS hook here).
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .map_err(|e| MainError::Serve(e.to_string()))?;
        return Ok(());
    }
    // Why: manual accept loop (like worker proxy) gives us peer certs for authz.
    let cert = state
        .config
        .tls
        .cert_file
        .clone()
        .unwrap_or_else(|| PathBuf::from("/nonexistent-cert.pem"));
    let key = state
        .config
        .tls
        .key_file
        .clone()
        .unwrap_or_else(|| PathBuf::from("/nonexistent-key.pem"));
    let pki = FilePki {
        cert,
        key,
        client_ca: state.config.tls.client_ca_file.clone(),
    };
    let tls_cfg = pki.server_config().map_err(MainError::Tls)?;
    // Why: validate rotation path at startup; failures abort instead of serving stale certs.
    pki.reload().map_err(MainError::Tls)?;
    serve_mtls(addr, tls_cfg, app).await
}

/// TLS accept loop that injects `PeerIdentity` per connection.
/// Why: handlers authorize writes by client cert without parsing TLS themselves.
async fn serve_mtls(
    addr: SocketAddr,
    tls_cfg: Arc<rustls::ServerConfig>,
    app: axum::Router,
) -> Result<(), MainError> {
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use hyper_util::server::conn::auto::Builder;
    use hyper_util::service::TowerToHyperService;
    use tokio_rustls::TlsAcceptor;

    let acceptor = TlsAcceptor::from(tls_cfg);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| MainError::Bind(e.to_string()))?;
    tracing::info!(%addr, "cache listening with mTLS");
    loop {
        let (stream, peer_addr) = listener
            .accept()
            .await
            .map_err(|e| MainError::Serve(e.to_string()))?;
        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let Ok(tls_stream) = acceptor.accept(stream).await else {
                tracing::warn!(%peer_addr, "TLS handshake failed");
                return;
            };
            // Why: rustls already verified the chain; we only label the peer.
            let mut identity = {
                let (_io, conn) = tls_stream.get_ref();
                let leaf = conn.peer_certificates().and_then(|chain| chain.first());
                identity_from_leaf(leaf.map(std::convert::AsRef::as_ref))
            };
            identity.is_loopback = peer_addr.ip().is_loopback();
            tracing::debug!(
                %peer_addr,
                cn = ?identity.common_name,
                fp = %identity.fingerprint_hex,
                "mTLS peer"
            );
            // Why: per-connection layer so all requests share this peer's identity.
            let service = tower::ServiceBuilder::new()
                .layer(axum::Extension(identity))
                .service(app);
            let service = TowerToHyperService::new(service);
            if let Err(e) = Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(tls_stream), service)
                .await
            {
                tracing::warn!(%peer_addr, error = %e, "HTTPS connection failed");
            }
        });
    }
}
