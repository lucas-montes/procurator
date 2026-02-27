//! Cloud Hypervisor VMM backend implementation.
//!
//! Three types work together:
//!
//! - [`CloudHypervisor`] — per-VM REST client (implements [`Vmm`]).
//! - [`ChProcess`] — handle to one `cloud-hypervisor` OS process (implements [`VmmProcess`]).
//! - [`CloudHypervisorBackend`] — factory that spawns CH processes (implements [`VmmBackend`]).

use std::path::{Path, PathBuf};
use std::time::Duration;

use hyperlocal::{UnixClientExt, Uri as UnixUri};
use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};
use tracing::debug;

use crate::dto::{VmError, VmSpec};
use crate::vmm::{Vmm, VmmBackend, VmmProcess};

// ─── Per-VM REST client ───────────────────────────────────────────────────

/// Stateless HTTP client to a single CH unix socket.
/// One instance per VM (created by [`CloudHypervisorBackend::spawn`]).
pub struct CloudHypervisor {
    /// Path to the unix socket for the cloud-hypervisor API
    socket_path: PathBuf,

    /// HTTP client configured for unix socket communication
    client: hyper::Client<hyperlocal::UnixConnector>,
}

impl CloudHypervisor {
    /// Create a new CloudHypervisor VMM instance
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

}

/// Cloud Hypervisor specific error types
#[derive(Debug)]
pub enum Error {
    Communication(String),
    VmNotFound(String),
    VmAlreadyExists(String),
    InvalidConfig(String),
    OperationFailed(String),
    Serialization(serde_json::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Communication(msg) => write!(f, "Communication error: {}", msg),
            Error::VmNotFound(id) => write!(f, "VM not found: {}", id),
            Error::VmAlreadyExists(id) => write!(f, "VM already exists: {}", id),
            Error::InvalidConfig(msg) => write!(f, "Invalid configuration: {}", msg),
            Error::OperationFailed(msg) => write!(f, "Operation failed: {}", msg),
            Error::Serialization(err) => write!(f, "Serialization error: {}", err),
            Error::Io(err) => write!(f, "IO error: {}", err),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Serialization(err) => Some(err),
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Serialization(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

impl Vmm for CloudHypervisor {
    type Config = ChVmConfig;
    type Info = ChVmInfo;
    type Error = Error;

    async fn create(&self, config: Self::Config) -> Result<(), Self::Error> {
        let body = serde_json::to_string(&config)?;

        let uri = self.build_uri("/api/v1/vm.create");
        let req = hyper::Request::builder()
            .method(hyper::Method::PUT)
            .uri(uri)
            .header("Content-Type", "application/json")
            .body(hyper::Body::from(body))
            .map_err(|e| Error::Communication(e.to_string()))?;

        let resp = self
            .client
            .request(req)
            .await
            .map_err(|e| Error::Communication(e.to_string()))?;

        if !resp.status().is_success() {
            let body_bytes = hyper::body::to_bytes(resp.into_body())
                .await
                .map_err(|e| Error::Communication(e.to_string()))?;
            let error_msg = String::from_utf8_lossy(&body_bytes);
            return Err(Error::OperationFailed(format!(
                "Failed to create VM: {}",
                error_msg
            )));
        }

        Ok(())
    }

    async fn boot(&self) -> Result<(), Self::Error> {
        let uri = self.build_uri("/api/v1/vm.boot");
        let req = hyper::Request::builder()
            .method(hyper::Method::PUT)
            .uri(uri)
            .body(hyper::Body::empty())
            .map_err(|e| Error::Communication(e.to_string()))?;

        let resp = self
            .client
            .request(req)
            .await
            .map_err(|e| Error::Communication(e.to_string()))?;

        if !resp.status().is_success() {
            let body_bytes = hyper::body::to_bytes(resp.into_body())
                .await
                .map_err(|e| Error::Communication(e.to_string()))?;
            let error_msg = String::from_utf8_lossy(&body_bytes);
            return Err(Error::OperationFailed(format!(
                "Failed to boot VM: {}",
                error_msg
            )));
        }

        Ok(())
    }

    async fn shutdown(&self) -> Result<(), Self::Error> {
        let uri = self.build_uri("/api/v1/vm.shutdown");
        let req = hyper::Request::builder()
            .method(hyper::Method::PUT)
            .uri(uri)
            .body(hyper::Body::empty())
            .map_err(|e| Error::Communication(e.to_string()))?;

        let resp = self
            .client
            .request(req)
            .await
            .map_err(|e| Error::Communication(e.to_string()))?;

        if !resp.status().is_success() {
            let body_bytes = hyper::body::to_bytes(resp.into_body())
                .await
                .map_err(|e| Error::Communication(e.to_string()))?;
            let error_msg = String::from_utf8_lossy(&body_bytes);
            return Err(Error::OperationFailed(format!(
                "Failed to shutdown VM: {}",
                error_msg
            )));
        }

        Ok(())
    }

    async fn delete(&self) -> Result<(), Self::Error> {
        let uri = self.build_uri("/api/v1/vm.delete");
        let req = hyper::Request::builder()
            .method(hyper::Method::PUT)
            .uri(uri)
            .body(hyper::Body::empty())
            .map_err(|e| Error::Communication(e.to_string()))?;

        let resp = self
            .client
            .request(req)
            .await
            .map_err(|e| Error::Communication(e.to_string()))?;

        if !resp.status().is_success() {
            let body_bytes = hyper::body::to_bytes(resp.into_body())
                .await
                .map_err(|e| Error::Communication(e.to_string()))?;
            let error_msg = String::from_utf8_lossy(&body_bytes);
            return Err(Error::OperationFailed(format!(
                "Failed to delete VM: {}",
                error_msg
            )));
        }

        Ok(())
    }

    async fn info(&self) -> Result<Self::Info, Self::Error> {
        let uri = self.build_uri("/api/v1/vm.info");
        let req = hyper::Request::builder()
            .method(hyper::Method::GET)
            .uri(uri)
            .body(hyper::Body::empty())
            .map_err(|e| Error::Communication(e.to_string()))?;

        let resp = self
            .client
            .request(req)
            .await
            .map_err(|e| Error::Communication(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(Error::OperationFailed(
                "Failed to get VM info".into(),
            ));
        }

        let body_bytes = hyper::body::to_bytes(resp.into_body())
            .await
            .map_err(|e| Error::Communication(e.to_string()))?;

        let ch_info = serde_json::from_slice(&body_bytes)?;

        Ok(ch_info)
    }

    async fn pause(&self) -> Result<(), Self::Error> {
        let uri = self.build_uri("/api/v1/vm.pause");
        let req = hyper::Request::builder()
            .method(hyper::Method::PUT)
            .uri(uri)
            .body(hyper::Body::empty())
            .map_err(|e| Error::Communication(e.to_string()))?;

        let resp = self
            .client
            .request(req)
            .await
            .map_err(|e| Error::Communication(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(Error::OperationFailed("Failed to pause VM".into()));
        }

        Ok(())
    }

    async fn resume(&self) -> Result<(), Self::Error> {
        let uri = self.build_uri("/api/v1/vm.resume");
        let req = hyper::Request::builder()
            .method(hyper::Method::PUT)
            .uri(uri)
            .body(hyper::Body::empty())
            .map_err(|e| Error::Communication(e.to_string()))?;

        let resp = self
            .client
            .request(req)
            .await
            .map_err(|e| Error::Communication(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(Error::OperationFailed("Failed to resume VM".into()));
        }

        Ok(())
    }

    async fn counters(&self) -> Result<Self::Info, Self::Error> {
        let uri = self.build_uri("/api/v1/vm.counters");
        let req = hyper::Request::builder()
            .method(hyper::Method::GET)
            .uri(uri)
            .body(hyper::Body::empty())
            .map_err(|e| Error::Communication(e.to_string()))?;

        let resp = self
            .client
            .request(req)
            .await
            .map_err(|e| Error::Communication(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(Error::OperationFailed("Failed to get VM counters".into()));
        }

        let body_bytes = hyper::body::to_bytes(resp.into_body())
            .await
            .map_err(|e| Error::Communication(e.to_string()))?;

        let counters = serde_json::from_slice(&body_bytes)?;
        Ok(counters)
    }

    async fn ping(&self) -> Result<(), Self::Error> {
        let uri = self.build_uri("/api/v1/vmm.ping");
        let req = hyper::Request::builder()
            .method(hyper::Method::GET)
            .uri(uri)
            .body(hyper::Body::empty())
            .map_err(|e| Error::Communication(e.to_string()))?;

        let resp = self
            .client
            .request(req)
            .await
            .map_err(|e| Error::Communication(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(Error::Communication("VMM ping failed".into()));
        }

        Ok(())
    }
}

// Cloud Hypervisor API data structures — all owned, no lifetimes.
// These get serialized to JSON for CH REST calls. The allocation cost
// is negligible vs spawning a process.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChVmConfig {
    pub cpus: ChCpusConfig,
    pub memory: ChMemoryConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<ChPayloadConfig>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub disks: Vec<ChDiskConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Vec<ChNetConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rng: Option<ChRngConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub console: Option<ChConsoleConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<ChSerialConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChCpusConfig {
    pub boot_vcpus: u8,
    pub max_vcpus: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChMemoryConfig {
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChPayloadConfig {
    pub kernel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cmdline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initramfs: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChDiskConfig {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readonly: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChNetConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChRngConfig {
    pub src: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChConsoleConfig {
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChSerialConfig {
    pub mode: String,
}

#[derive(Debug, Deserialize)]
pub struct ChVmInfo {
    pub state: String,
    pub config: ChVmConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_actual_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ChVmCounters {
    #[serde(rename = "block")]
    pub block: Option<ChBlockCounters>,
    #[serde(rename = "net")]
    pub net: Option<ChNetCounters>,
}

#[derive(Debug, Deserialize)]
pub struct ChBlockCounters {
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ChNetCounters {
    pub rx_bytes: Option<u64>,
    pub tx_bytes: Option<u64>,
}

// ─── Process handle ───────────────────────────────────────────────────────

/// Handle to one `cloud-hypervisor` OS process.
///
/// Owns the [`Child`] and the socket path. Cleans up both on [`VmmProcess::cleanup`].
pub struct ChProcess {
    child: Child,
    socket_path: PathBuf,
}

impl VmmProcess for ChProcess {
    async fn kill(&mut self) -> Result<(), VmError> {
        self.child
            .kill()
            .await
            .map_err(|e| VmError::ProcessFailed(format!("Failed to kill CH process: {e}")))
    }

    async fn cleanup(&mut self) -> Result<(), VmError> {
        if self.socket_path.exists() {
            let _ = tokio::fs::remove_file(&self.socket_path).await;
        }
        Ok(())
    }
}

// ─── Backend factory ──────────────────────────────────────────────────────

/// Configuration for [`CloudHypervisorBackend`].
pub struct CloudHypervisorConfig {
    /// Directory where VM sockets are created (e.g. `/tmp/procurator/vms/`)
    pub socket_dir: PathBuf,
    /// Path to the `cloud-hypervisor` binary
    pub ch_binary: PathBuf,
    /// How long to wait for a CH socket to appear after spawning
    pub socket_timeout: Duration,
}

impl Default for CloudHypervisorConfig {
    fn default() -> Self {
        Self {
            socket_dir: PathBuf::from("/tmp/procurator/vms"),
            ch_binary: PathBuf::from("cloud-hypervisor"),
            socket_timeout: Duration::from_secs(5),
        }
    }
}

/// Factory that spawns `cloud-hypervisor` processes and creates
/// [`CloudHypervisor`] REST clients.
///
/// This is the production implementation of [`VmmBackend`].
pub struct CloudHypervisorBackend {
    config: CloudHypervisorConfig,
}

impl CloudHypervisorBackend {
    pub fn new(config: CloudHypervisorConfig) -> Self {
        Self { config }
    }

    /// Poll for a unix socket to appear on disk with exponential backoff.
    async fn wait_for_socket(path: &Path, timeout: Duration) -> Result<(), VmError> {
        let start = std::time::Instant::now();
        let mut delay = Duration::from_millis(10);

        while start.elapsed() < timeout {
            if path.exists() {
                debug!(path = %path.display(), "Socket ready");
                return Ok(());
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_millis(500));
        }

        Err(VmError::ProcessFailed(format!(
            "Socket {} did not appear within {:?}",
            path.display(),
            timeout,
        )))
    }
}

impl VmmBackend for CloudHypervisorBackend {
    type Client = CloudHypervisor;
    type Process = ChProcess;

    async fn spawn(
        &self,
        vm_id: &str,
    ) -> Result<(CloudHypervisor, ChProcess, PathBuf), VmError> {
        // 1. Ensure socket directory exists
        tokio::fs::create_dir_all(&self.config.socket_dir)
            .await
            .map_err(|e| VmError::ProcessFailed(format!("Failed to create socket dir: {e}")))?;

        // 2. Build socket path
        let socket_path = self.config.socket_dir.join(format!("{vm_id}.sock"));

        // 3. Clean up stale socket if present
        if socket_path.exists() {
            let _ = tokio::fs::remove_file(&socket_path).await;
        }

        // 4. Spawn the CH process
        let child = Command::new(&self.config.ch_binary)
            .arg("--api-socket")
            .arg(&socket_path)
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                VmError::ProcessFailed(format!(
                    "Failed to spawn {}: {e}",
                    self.config.ch_binary.display()
                ))
            })?;

        // 5. Wait for socket to appear
        Self::wait_for_socket(&socket_path, self.config.socket_timeout).await?;

        // 6. Create the REST client and process handle
        let client = CloudHypervisor::new(&socket_path);
        let process = ChProcess {
            child,
            socket_path: socket_path.clone(),
        };

        Ok((client, process, socket_path))
    }

    fn build_config(&self, spec: &VmSpec) -> ChVmConfig {
        let boot_vcpus = spec.cpu().ceil() as u8;

        // Use the explicit paths from the Nix-built vmSpec.
        // These are separate store paths for kernel, initrd, and disk image.
        let kernel_path = spec.kernel_path().to_string();
        let disk_path = spec.disk_image_path().to_string();
        let initrd_path = spec.initrd_path().map(|s| s.to_string());
        let cmdline = spec.cmdline().map(|s| s.to_string());

        ChVmConfig {
            cpus: ChCpusConfig {
                boot_vcpus,
                max_vcpus: boot_vcpus,
            },
            memory: ChMemoryConfig {
                size: spec.memory_bytes(),
            },
            payload: Some(ChPayloadConfig {
                kernel: kernel_path,
                cmdline,
                initramfs: initrd_path,
            }),
            disks: vec![ChDiskConfig {
                path: disk_path,
                readonly: Some(false),
                direct: None,
            }],
            net: None,
            rng: Some(ChRngConfig {
                src: "/dev/urandom".to_string(),
            }),
            console: Some(ChConsoleConfig {
                mode: "Off".to_string(),
            }),
            serial: Some(ChSerialConfig {
                mode: "Null".to_string(),
            }),
        }
    }
}
