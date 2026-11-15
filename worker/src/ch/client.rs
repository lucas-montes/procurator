//! Cloud Hypervisor VMM backend implementation.
//! It does the requests to the CH API and translates between our internal config and the CH API.

use std::path::PathBuf;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, Uri};
use hyper_util::client::legacy::Client as HyperClient;
use hyperlocal::{UnixClientExt, UnixConnector, Uri as UnixUri};

use serde::{Serialize, de::DeserializeOwned};
use tracing::{debug, info};

use super::{dtos::VmConfigRef, errors::Error};
use crate::msglog::{Endpoint, Msg};

/// Stateless HTTP client to a single CH unix socket.
pub struct Client {
    /// Path to the unix socket for the cloud-hypervisor API
    socket_path: PathBuf,
    /// HTTP client configured for unix socket communication
    http: HyperClient<UnixConnector, Full<Bytes>>,
    /// VM this process runs, named in message lines as `vmm:<id>`.
    vm_id: String,
}

// Request structure used to serialize snapshot requests to the CH API.
#[derive(Serialize)]
struct SnapshotRequest<'a> {
    destination_url: &'a str,
}

impl Client {
    /// Create a new Client VMM instance
    pub fn new(socket_path: impl Into<PathBuf>, vm_id: impl Into<String>) -> Self {
        let client: HyperClient<UnixConnector, Full<Bytes>> = HyperClient::unix();

        Self {
            socket_path: socket_path.into(),
            http: client,
            vm_id: vm_id.into(),
        }
    }

    /// Calls `/api/v1/<action>` and writes its message line (the callee
    /// is not ours, so the caller logs it).
    async fn call<R: DeserializeOwned>(
        &self,
        method: Method,
        action: &'static str,
        body: Bytes,
    ) -> Result<R, Error> {
        let mut msg =
            Msg::start("vmm", action, Endpoint::Myself, Endpoint::Vmm(&self.vm_id)).vm(&self.vm_id);
        let uri: Uri = UnixUri::new(&self.socket_path, &format!("/api/v1/{action}")).into();
        let result = request::<R>(uri, body, method, &self.http).await;
        match &result {
            Ok(_) => msg.ok(),
            Err(err) => msg.error(err),
        }
        result
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

        let resp: serde_json::Value = self
            .call(Method::PUT, "vm.create", Bytes::from(body))
            .await?;

        info!(?resp, "vm.create succeeded");
        Ok(())
    }

    pub async fn boot(&self) -> Result<(), Error> {
        debug!("vm.boot request");
        let resp: serde_json::Value = self.call(Method::PUT, "vm.boot", Bytes::new()).await?;

        info!(?resp, "vm.boot succeeded");
        Ok(())
    }

    pub async fn delete(&self) -> Result<(), Error> {
        let resp: serde_json::Value = self.call(Method::PUT, "vm.delete", Bytes::new()).await?;

        info!(?resp, "vm.delete succeeded");
        Ok(())
    }

    /// Pause the VM. Required before taking a snapshot or doing a consistent disk copy.
    pub async fn pause(&self) -> Result<(), Error> {
        let resp: serde_json::Value = self.call(Method::PUT, "vm.pause", Bytes::new()).await?;

        info!(?resp, "vm.pause succeeded");
        Ok(())
    }

    /// Resume a previously paused VM.
    pub async fn resume(&self) -> Result<(), Error> {
        let resp: serde_json::Value = self.call(Method::PUT, "vm.resume", Bytes::new()).await?;

        info!(?resp, "vm.resume succeeded");
        Ok(())
    }

    /// Cumulative per-device counters (`vm.counters`): net `rx_bytes` /
    /// `tx_bytes`, block `read_bytes` / `write_bytes`, keyed by device id.
    pub async fn counters(&self) -> Result<serde_json::Value, Error> {
        self.call(Method::GET, "vm.counters", Bytes::new()).await
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

        let resp: serde_json::Value = self
            .call(Method::PUT, "vm.snapshot", Bytes::from(body))
            .await?;

        info!(?resp, "vm.snapshot succeeded");
        Ok(())
    }
}

async fn request<R: DeserializeOwned>(
    uri: Uri,
    body: Bytes,
    method: Method,
    client: &HyperClient<UnixConnector, Full<Bytes>>,
) -> Result<R, Error> {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json")
        .body(Full::new(body))
        .map_err(|e| Error::Communication(e.to_string()))?;

    let resp = client
        .request(req)
        .await
        .map_err(|e| Error::Communication(e.to_string()))?;

    let status = resp.status();

    let bytes = resp
        .into_body()
        .collect()
        .await
        .map_err(|e| Error::Communication(e.to_string()))?
        .to_bytes();

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
