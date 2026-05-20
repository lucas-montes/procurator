use std::{
    io::{self, BufReader},
    path::Path,
    sync::Arc,
};

use axum::{
    Router,
    body::Body,
    routing::{any, get, post},
};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
    service::TowerToHyperService,
};
use tokio::{net::TcpListener, task};
use tokio_rustls::{TlsAcceptor, rustls};
use tracing::{error, info};

use crate::{
    config::ProxyConfig,
    vmm::{Factory, Reader, Registry},
};

use super::{ProxyRuntimeSettings, ProxyState, auth_bootstrap, auth_logout, proxy_handler};

/// # Errors
///
/// - if TLS cert/key files cannot be read or are invalid
/// - if the TCP listener fails to bind to the configured address
/// - if the TLS acceptor or hyper connection serving encounters an error
///
pub async fn serve_tls_proxy<F: Factory>(
    proxy_config: ProxyConfig,
    registry: Registry<F, Reader>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tls_config = load_tls_config(&proxy_config)?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));
    let client: Client<HttpConnector, Body> =
        Client::builder(TokioExecutor::new()).build(HttpConnector::new());
    let runtime_settings = ProxyRuntimeSettings::from_timeout_millis(
        proxy_config.upstream_request_timeout_millis.get(),
    );
    let state = Arc::new(ProxyState {
        registry,
        client,
        jwt_hs256_secret: proxy_config.jwt_hs256_secret.clone(),
        base_domain: proxy_config.base_domain.clone(),
        runtime_settings,
    });
    let app = Router::new()
        .route("/__pcr/auth", get(auth_bootstrap::<F>))
        .route("/__pcr/logout", post(auth_logout::<F>))
        .fallback(any(proxy_handler::<F>))
        .with_state(state);

    let listener = TcpListener::bind(proxy_config.public_listen_addr)
        .await
        .inspect_err(|err| {
            error!(
                addr = %proxy_config.public_listen_addr,
                ?err,
                "Failed to bind proxy TLS listener"
            );
        })?;

    info!(
        addr = %proxy_config.public_listen_addr,
        "Starting worker's proxy TLS server"
    );

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let app = app.clone();

        task::spawn(async move {
            let Ok(tls_stream) = acceptor.accept(stream).await else {
                error!(peer_addr = %peer_addr, "TLS handshake failed for proxy client");
                return;
            };

            let service = TowerToHyperService::new(app);
            let io = TokioIo::new(tls_stream);
            if let Err(err) = Builder::new(TokioExecutor::new())
                .serve_connection(io, service)
                .await
            {
                error!(peer_addr = %peer_addr, ?err, "Proxy HTTPS connection failed");
            }
        });
    }
}

fn load_tls_config(proxy_config: &ProxyConfig) -> Result<rustls::ServerConfig, io::Error> {
    let cert_chain = load_certificates(&proxy_config.tls_cert_path)?;
    let private_key = load_private_key(&proxy_config.tls_key_path)?;

    rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, private_key)
        .map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid tls cert/key pair: {err}"),
            )
        })
}

fn load_certificates(
    path: &Path,
) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, io::Error> {
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let certs: Result<Vec<_>, _> = rustls_pemfile::certs(&mut reader).collect();
    let certs = certs.map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    if certs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("no certificates found in {}", path.display()),
        ));
    }

    Ok(certs)
}

fn load_private_key(path: &Path) -> Result<rustls::pki_types::PrivateKeyDer<'static>, io::Error> {
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);

    if let Some(key) = rustls_pemfile::private_key(&mut reader)? {
        return Ok(key);
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("no private key found in {}", path.display()),
    ))
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        net::SocketAddr,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn temp_file_path(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}.pem"))
    }

    #[test]
    fn rejects_missing_cert_file() {
        let config = ProxyConfig {
            public_listen_addr: SocketAddr::from(([127, 0, 0, 1], 8443)),
            tls_cert_path: std::path::PathBuf::from("/tmp/procurator2-missing-cert-file.pem"),
            tls_key_path: std::path::PathBuf::from("/tmp/procurator2-missing-key-file.pem"),
            base_domain: "vm.example.test".to_string(),
            jwt_hs256_secret: "secret".to_string(),
            upstream_request_timeout_millis: std::num::NonZeroU64::new(30_000).expect("non-zero"),
        };

        let result = load_tls_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_invalid_cert_contents() {
        let cert_path = temp_file_path("procurator2-invalid-cert");
        let key_path = temp_file_path("procurator2-invalid-key");

        let mut cert_file = std::fs::File::create(&cert_path).expect("create cert file");
        writeln!(cert_file, "not a cert").expect("write cert file");

        let mut key_file = std::fs::File::create(&key_path).expect("create key file");
        writeln!(key_file, "not a key").expect("write key file");

        let config = ProxyConfig {
            public_listen_addr: SocketAddr::from(([127, 0, 0, 1], 8443)),
            tls_cert_path: cert_path.clone(),
            tls_key_path: key_path.clone(),
            base_domain: "vm.example.test".to_string(),
            jwt_hs256_secret: "secret".to_string(),
            upstream_request_timeout_millis: std::num::NonZeroU64::new(30_000).expect("non-zero"),
        };

        let result = load_tls_config(&config);
        assert!(result.is_err());

        let _ = std::fs::remove_file(cert_path);
        let _ = std::fs::remove_file(key_path);
    }
}
