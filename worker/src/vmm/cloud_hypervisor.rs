//! Cloud Hypervisor VMM backend implementation
//!
//! Communicates with cloud-hypervisor via REST API over unix socket

use crate::vmm::Vmm;
use commands::common_capnp::vm_spec;

use hyperlocal::{UnixClientExt, Uri as UnixUri};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::borrow::Cow;

/// Cloud Hypervisor VMM implementation - stateless REST client
#[derive(Clone)]
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
pub enum ChError {
    Communication(String),
    VmNotFound(String),
    VmAlreadyExists(String),
    InvalidConfig(String),
    OperationFailed(String),
    Serialization(serde_json::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for ChError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChError::Communication(msg) => write!(f, "Communication error: {}", msg),
            ChError::VmNotFound(id) => write!(f, "VM not found: {}", id),
            ChError::VmAlreadyExists(id) => write!(f, "VM already exists: {}", id),
            ChError::InvalidConfig(msg) => write!(f, "Invalid configuration: {}", msg),
            ChError::OperationFailed(msg) => write!(f, "Operation failed: {}", msg),
            ChError::Serialization(err) => write!(f, "Serialization error: {}", err),
            ChError::Io(err) => write!(f, "IO error: {}", err),
        }
    }
}

impl std::error::Error for ChError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ChError::Serialization(err) => Some(err),
            ChError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for ChError {
    fn from(err: serde_json::Error) -> Self {
        ChError::Serialization(err)
    }
}

impl From<std::io::Error> for ChError {
    fn from(err: std::io::Error) -> Self {
        ChError::Io(err)
    }
}

impl Vmm for CloudHypervisor {
    type Config = ChVmConfig<'static>;
    type Info = ChVmInfo;
    type Error = ChError;

    async fn create(&self, config: Self::Config, boot: bool) -> Result<(), Self::Error> {
        let body = serde_json::to_string(&config)?;

        // Create VM via REST API
        let uri = self.build_uri("/api/v1/vm.create");
        let req = hyper::Request::builder()
            .method(hyper::Method::PUT)
            .uri(uri)
            .header("Content-Type", "application/json")
            .body(hyper::Body::from(body))
            .map_err(|e| ChError::Communication(e.to_string()))?;

        let resp = self
            .client
            .request(req)
            .await
            .map_err(|e| ChError::Communication(e.to_string()))?;

        if !resp.status().is_success() {
            let body_bytes = hyper::body::to_bytes(resp.into_body())
                .await
                .map_err(|e| ChError::Communication(e.to_string()))?;
            let error_msg = String::from_utf8_lossy(&body_bytes);
            return Err(ChError::OperationFailed(format!(
                "Failed to create VM: {}",
                error_msg
            )));
        }

        // Boot if requested
        if boot {
            let uri = self.build_uri("/api/v1/vm.boot");
            let req = hyper::Request::builder()
                .method(hyper::Method::PUT)
                .uri(uri)
                .body(hyper::Body::empty())
                .map_err(|e| ChError::Communication(e.to_string()))?;

            let resp = self
                .client
                .request(req)
                .await
                .map_err(|e| ChError::Communication(e.to_string()))?;

            if !resp.status().is_success() {
                return Err(ChError::OperationFailed("Failed to boot VM".into()));
            }
        }

        Ok(())
    }

    async fn delete(&self, vm_id: &str) -> Result<(), Self::Error> {
        // Try to shutdown first (may fail if already shut down, that's ok)
        let uri = self.build_uri("/api/v1/vm.shutdown");
        let req = hyper::Request::builder()
            .method(hyper::Method::PUT)
            .uri(uri)
            .body(hyper::Body::empty())
            .map_err(|e| ChError::Communication(e.to_string()))?;

        let _ = self.client.request(req).await;

        // Delete the VM
        let uri = self.build_uri("/api/v1/vm.delete");
        let req = hyper::Request::builder()
            .method(hyper::Method::PUT)
            .uri(uri)
            .body(hyper::Body::empty())
            .map_err(|e| ChError::Communication(e.to_string()))?;

        let resp = self
            .client
            .request(req)
            .await
            .map_err(|e| ChError::Communication(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(ChError::OperationFailed("Failed to delete VM".into()));
        }

        Ok(())
    }

    async fn info(&self, vm_id: &str) -> Result<Self::Info, Self::Error> {
        // Query VM info from cloud-hypervisor
        let uri = self.build_uri("/api/v1/vm.info");
        let req = hyper::Request::builder()
            .method(hyper::Method::GET)
            .uri(uri)
            .body(hyper::Body::empty())
            .map_err(|e| ChError::Communication(e.to_string()))?;

        let resp = self
            .client
            .request(req)
            .await
            .map_err(|e| ChError::Communication(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(ChError::OperationFailed(
                "Failed to get VM info".into(),
            ));
        }

        let body_bytes = hyper::body::to_bytes(resp.into_body())
            .await
            .map_err(|e| ChError::Communication(e.to_string()))?;

        // Deserialize - Cow will be Owned since we're deserializing from bytes
        let ch_info = serde_json::from_slice(&body_bytes)?;

        Ok(ch_info)
    }

    async fn list(&self) -> Result<Vec<String>, Self::Error> {
        // Cloud Hypervisor doesn't provide a list endpoint
        // This would need to be tracked externally or use a different approach
        Ok(vec![])
    }
}

// Cloud Hypervisor API data structures with lifetime support for zero-copy

#[derive(Debug, Serialize, Deserialize)]
pub struct ChVmConfig<'a> {
    pub cpus: ChCpusConfig,
    pub memory: ChMemoryConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<ChPayloadConfig<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(borrow)]
    pub disks: Vec<ChDiskConfig<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(borrow)]
    pub net: Option<Vec<ChNetConfig<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rng: Option<ChRngConfig<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub console: Option<ChConsoleConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<ChSerialConfig>,
}



#[derive(Debug, Serialize, Deserialize)]
pub struct ChCpusConfig {
    pub boot_vcpus: u8,
    pub max_vcpus: u8,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChMemoryConfig {
    pub size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChPayloadConfig<'a> {
    #[serde(borrow)]
    pub kernel: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(borrow)]
    pub cmdline: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(borrow)]
    pub initramfs: Option<Cow<'a, str>>,
}



#[derive(Debug, Serialize, Deserialize)]
pub struct ChDiskConfig<'a> {
    #[serde(borrow)]
    pub path: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readonly: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct: Option<bool>,
}



#[derive(Debug, Serialize, Deserialize)]
pub struct ChNetConfig<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(borrow)]
    pub tap: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(borrow)]
    pub ip: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(borrow)]
    pub mask: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(borrow)]
    pub mac: Option<Cow<'a, str>>,
}



#[derive(Debug, Serialize, Deserialize)]
pub struct ChRngConfig<'a> {
    #[serde(borrow)]
    pub src: Cow<'a, str>,
}



#[derive(Debug, Serialize, Deserialize)]
pub struct ChConsoleConfig {
    pub mode: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChSerialConfig {
    pub mode: String,
}

// Owned version of config for VM info responses
#[derive(Debug, Serialize, Deserialize)]
pub struct ChVmConfigOwned {
    pub cpus: ChCpusConfig,
    pub memory: ChMemoryConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<ChPayloadConfigOwned>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub disks: Vec<ChDiskConfigOwned>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Vec<ChNetConfigOwned>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rng: Option<ChRngConfigOwned>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub console: Option<ChConsoleConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<ChSerialConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChPayloadConfigOwned {
    pub kernel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cmdline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initramfs: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChDiskConfigOwned {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readonly: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChNetConfigOwned {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChRngConfigOwned {
    pub src: String,
}

#[derive(Debug, Deserialize)]
pub struct ChVmInfo {
    pub state: String,
    pub config: ChVmConfigOwned,
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


impl<'a> TryFrom<vm_spec::Reader<'a>> for ChVmConfig<'a> {
    type Error = capnp::Error;

    fn try_from(spec: vm_spec::Reader<'a>) -> Result<Self, Self::Error> {
        let cpu = spec.get_cpu();
        let memory_bytes = spec.get_memory_bytes();
        let store_path = spec.get_store_path()?.to_str()?;

        // Convert fractional CPU to boot_vcpus (round up)
        let boot_vcpus = cpu.ceil() as u8;

        Ok(ChVmConfig {
            cpus: ChCpusConfig {
                boot_vcpus,
                max_vcpus: boot_vcpus,
            },
            memory: ChMemoryConfig {
                size: memory_bytes,
            },
            payload: Some(ChPayloadConfig {
                kernel: Cow::Borrowed(store_path),
                cmdline: None, // Could be populated from spec if needed
                initramfs: None,
            }),
            disks: Vec::new(), // TODO: Map from spec if needed
            net: None, // TODO: Map from networkAllowedDomains
            rng: Some(ChRngConfig {
                src: Cow::Borrowed("/dev/urandom"),
            }),
            console: Some(ChConsoleConfig {
                mode: "Off".to_string(),
            }),
            serial: Some(ChSerialConfig {
                mode: "Null".to_string(),
            }),
        })
    }
}
