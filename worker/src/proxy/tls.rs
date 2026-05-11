use std::{
    io::{self, BufReader},
    path::Path,
    sync::Arc,
};

use hyper::{
    Body, Client, Request, Response, client::HttpConnector, server::conn::Http, service::service_fn,
};
use tokio::{net::TcpListener, task};
use tokio_rustls::{TlsAcceptor, rustls};
use tracing::{error, info};

use crate::{
    config::ProxyConfig,
    vmm::{Factory, Reader, Registry},
};

use super::{ProxyRuntimeSettings, proxy_vm_request};

/// # Errors
///
/// - if TLS cert/key files cannot be read or are invalid
/// - if the TCP listener fails to bind to the configured address
/// - if the TLS acceptor or hyper connection serving encounters an error
///
pub async fn serve_tls_proxy<F: Factory>(
    proxy_config: ProxyConfig,
    registry: Registry<F, Reader>,
) -> Result<(), Box<dyn std::error::Error>> {
    let tls_config = load_tls_config(&proxy_config)?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));
    let client: Client<HttpConnector, Body> = Client::new();
    let opencode_upstream_port = proxy_config.opencode_upstream_port;
    let runtime_settings = ProxyRuntimeSettings::from_timeout_millis(
        proxy_config
            .timeouts
            .upstream_request_timeout_millis
            .map(std::num::NonZero::get),
    );

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
        "Starting worker proxy TLS listener"
    );

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let registry = registry.clone();
        let client = client.clone();
        let jwt_hs256_secret = proxy_config.jwt_hs256_secret.clone();

        task::spawn_local(async move {
            let Ok(tls_stream) = acceptor.accept(stream).await else {
                error!(peer_addr = %peer_addr, "TLS handshake failed for proxy client");
                return;
            };

            let service = service_fn(move |request: Request<Body>| {
                let registry = registry.clone();
                let client = client.clone();
                let jwt_hs256_secret = jwt_hs256_secret.clone();
                async move {
                    Ok::<Response<Body>, std::convert::Infallible>(
                        proxy_vm_request(
                            &registry,
                            &client,
                            &jwt_hs256_secret,
                            request,
                            runtime_settings,
                            opencode_upstream_port,
                        )
                        .await,
                    )
                }
            });

            if let Err(err) = Http::new().serve_connection(tls_stream, service).await {
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
    use crate::config::ProxyTimeouts;

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
            opencode_upstream_port: 4096,
            public_listen_addr: SocketAddr::from(([127, 0, 0, 1], 8443)),
            tls_cert_path: std::path::PathBuf::from("/tmp/procurator2-missing-cert-file.pem"),
            tls_key_path: std::path::PathBuf::from("/tmp/procurator2-missing-key-file.pem"),
            jwt_hs256_secret: "secret".to_string(),
            timeouts: ProxyTimeouts::default(),
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
            opencode_upstream_port: 4096,
            public_listen_addr: SocketAddr::from(([127, 0, 0, 1], 8443)),
            tls_cert_path: cert_path.clone(),
            tls_key_path: key_path.clone(),
            jwt_hs256_secret: "secret".to_string(),
            timeouts: ProxyTimeouts::default(),
        };

        let result = load_tls_config(&config);
        assert!(result.is_err());

        let _ = std::fs::remove_file(cert_path);
        let _ = std::fs::remove_file(key_path);
    }
}
