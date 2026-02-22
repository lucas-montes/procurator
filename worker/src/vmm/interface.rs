//! VMM abstraction layer for managing virtual machines
//!
//! This module provides a pluggable interface for different VMM backends (cloud-hypervisor, etc.)

use serde::{Deserialize, Serialize};


/// Errors that can occur during VMM operations
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

pub type Result<T> = std::result::Result<T, Error>;

/// Configuration for creating a new VM
#[derive(Debug, Serialize, Deserialize)]
pub struct VmConfig {
    /// Unique identifier for the VM
    id: String,

    /// Number of vCPUs
    cpus: u8,

    /// Memory size in MB
    memory_mb: u64,

    /// Path to kernel image
    kernel_path: String,

    /// Kernel command line
    cmdline: Option<String>,

    /// Disk image paths
    disks: Vec<String>,

    /// Network configuration
    net: Option<NetworkConfig>,
}

impl VmConfig {
    pub fn new(
        id: String,
        cpus: u8,
        memory_mb: u64,
        kernel_path: String,
    ) -> Self {
        Self {
            id,
            cpus,
            memory_mb,
            kernel_path,
            cmdline: None,
            disks: Vec::new(),
            net: None,
        }
    }

}

/// Network configuration for a VM
#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkConfig {
    ip: Option<String>,
    mask: Option<String>,
    mac: Option<String>,
    tap: Option<String>,
}

impl NetworkConfig {
    pub fn new() -> Self {
        Self {
            ip: None,
            mask: None,
            mac: None,
            tap: None,
        }
    }

}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Runtime information about a VM
#[derive(Debug, Serialize, Deserialize)]
pub struct VmInfo {
    /// VM identifier
    id: String,

    /// Current state (Created, Running, Paused, Shutdown, etc.)
    state: VmState,

    /// Configuration used to create the VM
    config: VmConfig,

    /// Runtime metrics (CPU, memory, I/O)
    metrics: Option<VmMetrics>,
}

impl VmInfo {
    pub fn new(id: String, state: VmState, config: VmConfig) -> Self {
        Self {
            id,
            state,
            config,
            metrics: None,
        }
    }

}

/// VM lifecycle state
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VmState {
    /// VM is created but not yet booted
    Created,

    /// VM is running
    Running,

    /// VM is paused
    Paused,

    /// VM is shut down
    Shutdown,

    /// Unknown state
    Unknown,
}

/// Runtime metrics for a VM
#[derive(Debug, Serialize, Deserialize)]
pub struct VmMetrics {
    /// CPU usage percentage (0-100 per vCPU)
    cpu_usage: Vec<f64>,

    /// Memory usage in bytes
    memory_used_bytes: u64,

    /// Memory available in bytes
    memory_available_bytes: u64,

    /// Disk I/O read bytes
    disk_read_bytes: u64,

    /// Disk I/O write bytes
    disk_write_bytes: u64,

    /// Network RX bytes
    net_rx_bytes: u64,

    /// Network TX bytes
    net_tx_bytes: u64,
}

impl VmMetrics {
    pub fn new(
        cpu_usage: Vec<f64>,
        memory_used_bytes: u64,
        memory_available_bytes: u64,
        disk_read_bytes: u64,
        disk_write_bytes: u64,
        net_rx_bytes: u64,
        net_tx_bytes: u64,
    ) -> Self {
        Self {
            cpu_usage,
            memory_used_bytes,
            memory_available_bytes,
            disk_read_bytes,
            disk_write_bytes,
            net_rx_bytes,
            net_tx_bytes,
        }
    }
}

pub trait Vmm: Clone + 'static {
    /// Create and optionally boot a new VM
    async fn create(&self, config: VmConfig, boot: bool) -> Result<()>;

    /// Delete an existing VM (shutdown if running, then destroy)
    async fn delete(&self, vm_id: &str) -> Result<()>;

    /// Get information about a VM including its metrics
    async fn info(&self, vm_id: &str) -> Result<VmInfo>;

    /// List all VMs managed by this VMM
    async fn list(&self) -> Result<Vec<String>>;
}
