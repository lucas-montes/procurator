//! Cloud Hypervisor VMM backend implementation.
//! It does the requests to the CH API and translates between our internal config and the CH API.

use std::path::PathBuf;

use hyper::Uri;
use hyperlocal::{UnixClientExt, Uri as UnixUri};

use serde::{Serialize, de::DeserializeOwned};
use tracing::{debug, info};

use super::{dtos::VmConfigRef, errors::Error};

/// Stateless HTTP client to a single CH unix socket.
pub struct Client {
    /// Path to the unix socket for the cloud-hypervisor API
    socket_path: PathBuf,
    /// HTTP client configured for unix socket communication
    client: hyper::Client<hyperlocal::UnixConnector>,
}

// Request structure used to serialize snapshot requests to the CH API.
#[derive(Serialize)]
struct SnapshotRequest<'a> {
    destination_url: &'a str,
}

impl Client {
    /// Create a new Client VMM instance
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        let client = hyper::Client::unix();

        Self {
            socket_path: socket_path.into(),
            client,
        }
    }

    /// Build the unix socket URI for a given API endpoint
    fn build_uri(&self, endpoint: &str) -> hyper::Uri {
        UnixUri::new(&self.socket_path, endpoint).into()
    }

    /// Kill the client and VM returning the socket path for cleanup.
    pub async fn kill(self) -> Result<PathBuf, Error> {
        self.delete().await?;
        Ok(self.socket_path)
    }

    /// Create a new VM with the given configuration.
    pub async fn create(&self, config: &VmConfigRef<'_>) -> Result<(), Error> {
        let body = serde_json::to_string(config)?;
        debug!(config_json = %body, "vm.create request");

        let uri = self.build_uri("/api/v1/vm.create");
        let resp =
            request::<serde_json::Value>(uri, body, hyper::Method::PUT, &self.client).await?;

        info!(?resp, "vm.create succeeded");
        Ok(())
    }

    pub async fn boot(&self) -> Result<(), Error> {
        debug!("vm.boot request");
        let uri = self.build_uri("/api/v1/vm.boot");
        let resp = request::<serde_json::Value>(
            uri,
            hyper::Body::empty(),
            hyper::Method::PUT,
            &self.client,
        )
        .await?;

        info!(?resp, "vm.boot succeeded");
        Ok(())
    }

    pub async fn delete(&self) -> Result<(), Error> {
        let uri = self.build_uri("/api/v1/vm.delete");
        let resp = request::<serde_json::Value>(
            uri,
            hyper::Body::empty(),
            hyper::Method::PUT,
            &self.client,
        )
        .await?;

        info!(?resp, "vm.delete succeeded");
        Ok(())
    }

    /// Pause the VM. Required before taking a snapshot or doing a consistent disk copy.
    pub async fn pause(&self) -> Result<(), Error> {
        let uri = self.build_uri("/api/v1/vm.pause");
        let resp = request::<serde_json::Value>(
            uri,
            hyper::Body::empty(),
            hyper::Method::PUT,
            &self.client,
        )
        .await?;

        info!(?resp, "vm.pause succeeded");
        Ok(())
    }

    /// Resume a previously paused VM.
    pub async fn resume(&self) -> Result<(), Error> {
        let uri = self.build_uri("/api/v1/vm.resume");
        let resp = request::<serde_json::Value>(
            uri,
            hyper::Body::empty(),
            hyper::Method::PUT,
            &self.client,
        )
        .await?;

        info!(?resp, "vm.resume succeeded");
        Ok(())
    }

    /// Take a CH snapshot (memory + device state) into `destination_url`.
    ///
    /// The URL must be a `file://` URL pointing to an existing directory; CH will write
    /// `config.json`, `memory-ranges` and `state.json` inside it.
    /// The VM **must be paused** before calling this.
    pub async fn snapshot(&self, destination_url: &str) -> Result<(), Error> {
        let req = SnapshotRequest { destination_url };
        let body = serde_json::to_string(&req)?;
        debug!(%destination_url, "vm.snapshot request");

        let uri = self.build_uri("/api/v1/vm.snapshot");
        let resp =
            request::<serde_json::Value>(uri, body, hyper::Method::PUT, &self.client).await?;

        info!(?resp, "vm.snapshot succeeded");
        Ok(())
    }
}

async fn request<R: DeserializeOwned>(
    uri: Uri,
    body: impl Into<hyper::Body>,
    method: hyper::Method,
    client: &hyper::Client<hyperlocal::UnixConnector>,
) -> Result<R, Error> {
    let req = hyper::Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json")
        .body(body.into())
        .map_err(|e| Error::Communication(e.to_string()))?;

    let resp = client
        .request(req)
        .await
        .map_err(|e| Error::Communication(e.to_string()))?;

    let status = resp.status();

    let bytes = hyper::body::to_bytes(resp.into_body())
        .await
        .map_err(|e| Error::Communication(e.to_string()))?;

    if !status.is_success() {
        let msg = String::from_utf8_lossy(&bytes);
        //TODO: map the errors and so on into enums
        return Err(Error::OperationFailed(msg.to_string()));
    }

    if bytes.is_empty() {
        return serde_json::from_str::<R>("{}").map_err(|e| Error::Communication(e.to_string()));
    }

    serde_json::from_slice::<R>(&bytes).map_err(|e| Error::Communication(e.to_string()))
}
