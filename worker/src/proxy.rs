use std::net::SocketAddr;

use axum::{
    Router,
    extract::{Path, State},
    response::Response,
};
use axum_server;
use tokio::net::TcpListener;
use tracing::{debug, error, info, warn};

use crate::database::Database;

/// Creates an HTTP proxy router that forwards requests to VMs based on path prefix.
///
/// Path format: `/vm/<vm_id>/*path` forwards to `http://<vm_ip>:4096/<path>`
pub fn create_proxy_router(db: Database) -> Router {
    Router::new().fallback(handler).with_state(db)
}

async fn handler(
    Path((vm_id, remaining)): Path<(String, String)>,
    State(db): State<Database>,
    request: axum::extract::Request,
) -> Response {
    // Look up the VM IP from the database
    let vm_ip = match db.get_ip_by_vm_id(&vm_id).await {
        Ok(Some(ip)) => ip.to_string(),
        Ok(None) => {
            return Response::builder()
                .status(axum::http::StatusCode::NOT_FOUND)
                .body(axum::body::Body::from(format!("VM {} not found", vm_id)))
                .unwrap();
        }
        Err(e) => {
            error!(vm_id = %vm_id, error = %e, "Database error looking up VM");
            return Response::builder()
                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::from("Internal Server Error"))
                .unwrap();
        }
    };

    let path_and_query = if remaining.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", remaining)
    };

    let target_url = format!("http://{}:4096{}", vm_ip, path_and_query);

    debug!(vm_id = %vm_id, target = %target_url, "Proxying request to VM");

    // Forward the request to the VM using reqwest
    let client = reqwest::Client::new();

    let method = request.method().clone();
    let headers = request.headers().clone();

    // Read the request body
    let body_bytes = match axum::body::to_bytes(request.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(e) => {
            error!(vm_id = %vm_id, error = %e, "Failed to read request body");
            return Response::builder()
                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::from("Internal Server Error"))
                .unwrap();
        }
    };

    let mut proxy_request = client.request(method, target_url.as_str());

    // Copy headers (skip host header to avoid issues)
    for (key, value) in headers.iter() {
        if key.as_str() != "host" {
            proxy_request = proxy_request.header(key.as_str(), value.to_str().unwrap_or(""));
        }
    }

    // Send the request with body
    let response = match proxy_request.body(body_bytes).send().await {
        Ok(resp) => resp,
        Err(e) => {
            error!(vm_id = %vm_id, error = %e, "Failed to proxy request to VM");
            return Response::builder()
                .status(axum::http::StatusCode::BAD_GATEWAY)
                .body(axum::body::Body::from("Bad Gateway"))
                .unwrap();
        }
    };

    // Convert response
    let status = axum::http::StatusCode::from_u16(response.status().as_u16())
        .unwrap_or(axum::http::StatusCode::OK);
    let mut response_builder = Response::builder().status(status);

    // Copy response headers
    let resp_headers = response.headers();
    for (key, value) in resp_headers.iter() {
        response_builder = response_builder.header(key.as_str(), value.to_str().unwrap_or(""));
    }

    let body_bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            error!(vm_id = %vm_id, error = %e, "Failed to read response from VM");
            return Response::builder()
                .status(axum::http::StatusCode::BAD_GATEWAY)
                .body(axum::body::Body::from("Bad Gateway"))
                .unwrap();
        }
    };

    response_builder
        .body(axum::body::Body::from(body_bytes))
        .unwrap()
}

/// Starts the HTTP proxy server as a tokio task.
///
/// - `listen_addr`: Address to listen on (e.g., "0.0.0.0:8443")
/// - `db`: Database to look up VM IPs
/// - `enable_tls`: Whether to use TLS
/// - `tls_cert_path`: Path to TLS certificate (optional, generates self-signed if None)
/// - `tls_key_path`: Path to TLS key (optional)
pub async fn start_proxy_server(
    listen_addr: SocketAddr,
    db: Database,
    enable_tls: bool,
    tls_cert_path: Option<String>,
    tls_key_path: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let app = create_proxy_router(db);

    info!(addr = %listen_addr, tls = enable_tls, "Starting HTTP proxy server");

    if enable_tls {
        // Use TLS
        let (cert, key) = match (tls_cert_path, tls_key_path) {
            (Some(cert_path), Some(key_path)) => {
                // Load from provided paths
                let cert = tokio::fs::read(cert_path).await?;
                let key = tokio::fs::read(key_path).await?;
                (cert, key)
            }
            _ => {
                // Generate self-signed cert
                warn!("No TLS cert/key provided, generating self-signed certificate");
                let (cert, key) = generate_self_signed_cert()?;
                (cert, key)
            }
        };

        let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem(cert, key).await?;
        axum_server::tls_rustls::bind_rustls(listen_addr, tls_config)
            .serve(app.into_make_service())
            .await?;
    } else {
        // Plain HTTP
        let listener = TcpListener::bind(listen_addr).await?;
        axum::serve(listener, app.into_make_service()).await?;
    }

    Ok(())
}

/// Generates a self-signed TLS certificate for localhost.
fn generate_self_signed_cert() -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error>> {
    use rcgen::{CertifiedKey, generate_simple_self_signed};

    let subject_alt_names = vec!["localhost".to_string()];
    let CertifiedKey { cert, signing_key } = generate_simple_self_signed(subject_alt_names)?;

    let cert_pem = cert.pem();
    let key_pem = signing_key.serialize_pem();

    Ok((cert_pem.into_bytes(), key_pem.into_bytes()))
}
