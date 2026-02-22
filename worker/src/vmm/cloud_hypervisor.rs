//! Cloud Hypervisor VMM backend implementation
//!
//! Communicates with cloud-hypervisor via REST API over unix socket

use crate::vmm::{
    NetworkConfig, VmConfig, VmInfo, VmMetrics, VmState, Vmm, Error, Result,
};

use hyperlocal::{UnixClientExt, Uri as UnixUri};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

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

    /// Convert VmConfig to cloud-hypervisor's JSON payload
    fn to_ch_config(&self, config: &VmConfig) -> ChVmConfig {
        ChVmConfig {
            cpus: ChCpusConfig {
                boot_vcpus: config.cpus(),
                max_vcpus: config.cpus(),
            },
            memory: ChMemoryConfig {
                size: config.memory_mb() * 1024 * 1024, // Convert MB to bytes
            },
            payload: Some(ChPayloadConfig {
                kernel: config.kernel_path().to_string(),
                cmdline: config.cmdline().map(|s| s.to_string()),
                initramfs: None,
            }),
            disks: config
                .disks()
                .iter()
                .map(|path| ChDiskConfig {
                    path: path.clone(),
                    readonly: Some(false),
                    direct: Some(true),
                })
                .collect(),
            net: config.net().map(|net| {
                vec![ChNetConfig {
                    tap: net.tap().map(|s| s.to_string()),
                    ip: net.ip().map(|s| s.to_string()),
                    mask: net.mask().map(|s| s.to_string()),
                    mac: net.mac().map(|s| s.to_string()),
                }]
            }),
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

impl Vmm for CloudHypervisor {
    async fn create(&self, config: VmConfig, boot: bool) -> Result<()> {
        // Prepare the cloud-hypervisor config
        let ch_config = self.to_ch_config(&config);
        let body = serde_json::to_string(&ch_config)?;

        // Create VM via REST API
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

        // Boot if requested
        if boot {
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
                return Err(Error::OperationFailed("Failed to boot VM".into()));
            }
        }

        Ok(())
    }

    async fn delete(&self, vm_id: &str) -> Result<()> {
        // Try to shutdown first (may fail if already shut down, that's ok)
        let uri = self.build_uri("/api/v1/vm.shutdown");
        let req = hyper::Request::builder()
            .method(hyper::Method::PUT)
            .uri(uri)
            .body(hyper::Body::empty())
            .map_err(|e| Error::Communication(e.to_string()))?;

        let _ = self.client.request(req).await;

        // Delete the VM
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
            return Err(Error::OperationFailed("Failed to delete VM".into()));
        }

        Ok(())
    }

    async fn info(&self, vm_id: &str) -> Result<VmInfo> {
        // Query VM info from cloud-hypervisor
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

        let ch_info: ChVmInfo = serde_json::from_slice(&body_bytes)?;

        // Extract config from response
        let mut config = VmConfig::new(
            vm_id.to_string(),
            ch_info.config.cpus.boot_vcpus,
            ch_info.config.memory.size / 1024 / 1024,
            ch_info.config.payload.as_ref().map(|p| p.kernel.clone()).unwrap_or_default(),
        );

        if let Some(ref payload) = ch_info.config.payload {
            if let Some(ref cmdline) = payload.cmdline {
                config = config.with_cmdline(cmdline.clone());
            }
        }

        config = config.with_disks(
            ch_info.config.disks.iter().map(|d| d.path.clone()).collect()
        );

        if let Some(nets) = ch_info.config.net.as_ref() {
            if let Some(n) = nets.first() {
                let mut net_config = NetworkConfig::new();
                if let Some(ref ip) = n.ip {
                    net_config = net_config.with_ip(ip.clone());
                }
                if let Some(ref mask) = n.mask {
                    net_config = net_config.with_mask(mask.clone());
                }
                if let Some(ref mac) = n.mac {
                    net_config = net_config.with_mac(mac.clone());
                }
                if let Some(ref tap) = n.tap {
                    net_config = net_config.with_tap(tap.clone());
                }
                config = config.with_network(net_config);
            }
        }

        // Query VM counters for metrics
        let uri = self.build_uri("/api/v1/vm.counters");
        let req = hyper::Request::builder()
            .method(hyper::Method::GET)
            .uri(uri)
            .body(hyper::Body::empty())
            .map_err(|e| Error::Communication(e.to_string()))?;

        let counters_resp = self.client.request(req).await;

        let metrics = if let Ok(resp) = counters_resp {
            if resp.status().is_success() {
                let body_bytes = hyper::body::to_bytes(resp.into_body())
                    .await
                    .ok();

                body_bytes.and_then(|bytes| {
                    serde_json::from_slice::<ChVmCounters>(&bytes)
                        .ok()
                        .map(|counters| VmMetrics::new(
                            vec![0.0; config.cpus() as usize], // CH doesn't expose CPU %
                            0, // memory_used_bytes - Would need /proc parsing in guest
                            config.memory_mb() * 1024 * 1024,
                            counters
                                .block
                                .as_ref()
                                .and_then(|b| b.read_bytes)
                                .unwrap_or(0),
                            counters
                                .block
                                .as_ref()
                                .and_then(|b| b.write_bytes)
                                .unwrap_or(0),
                            counters
                                .net
                                .as_ref()
                                .and_then(|n| n.rx_bytes)
                                .unwrap_or(0),
                            counters
                                .net
                                .as_ref()
                                .and_then(|n| n.tx_bytes)
                                .unwrap_or(0),
                        ))
                })
            } else {
                None
            }
        } else {
            None
        };

        let mut vm_info = VmInfo::new(
            vm_id.to_string(),
            ch_info.state.parse().unwrap_or(VmState::Unknown),
            config,
        );

        if let Some(metrics) = metrics {
            vm_info = vm_info.with_metrics(metrics);
        }

        Ok(vm_info)
    }

    async fn list(&self) -> Result<Vec<String>> {
        // Cloud Hypervisor doesn't provide a list endpoint
        // This would need to be tracked externally or use a different approach
        Ok(vec![])
    }
}

// Cloud Hypervisor API data structures

#[derive(Serialize, Deserialize)]
struct ChVmConfig {
    cpus: ChCpusConfig,
    memory: ChMemoryConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<ChPayloadConfig>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    disks: Vec<ChDiskConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    net: Option<Vec<ChNetConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rng: Option<ChRngConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    console: Option<ChConsoleConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    serial: Option<ChSerialConfig>,
}

#[derive(Serialize, Deserialize)]
struct ChCpusConfig {
    boot_vcpus: u8,
    max_vcpus: u8,
}

#[derive(Serialize, Deserialize)]
struct ChMemoryConfig {
    size: u64,
}

#[derive(Serialize, Deserialize)]
struct ChPayloadConfig {
    kernel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cmdline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    initramfs: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ChDiskConfig {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    readonly: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    direct: Option<bool>,
}

#[derive(Serialize, Deserialize)]
struct ChNetConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    tap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mask: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mac: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ChRngConfig {
    src: String,
}

#[derive(Serialize, Deserialize)]
struct ChConsoleConfig {
    mode: String,
}

#[derive(Serialize, Deserialize)]
struct ChSerialConfig {
    mode: String,
}

#[derive(Deserialize)]
struct ChVmInfo {
    state: String,
    config: ChVmConfig,
}

#[derive(Deserialize)]
struct ChVmCounters {
    #[serde(rename = "block")]
    block: Option<ChBlockCounters>,
    #[serde(rename = "net")]
    net: Option<ChNetCounters>,
}

#[derive(Deserialize)]
struct ChBlockCounters {
    read_bytes: Option<u64>,
    write_bytes: Option<u64>,
}

#[derive(Deserialize)]
struct ChNetCounters {
    rx_bytes: Option<u64>,
    tx_bytes: Option<u64>,
}

impl std::str::FromStr for VmState {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "created" => Ok(VmState::Created),
            "running" => Ok(VmState::Running),
            "paused" => Ok(VmState::Paused),
            "shutdown" => Ok(VmState::Shutdown),
            _ => Ok(VmState::Unknown),
        }
    }
}
