use serde::de::DeserializeOwned;

use super::{Error, supervisor::CreateCommand};

/// This is the interface that should create process and talk to the hypervisor or whatever backend we are using to manage vms/vmms
/// The questions is, do all backends need a client and a process like cloud hypervisor?
pub trait Factory: Clone + 'static + std::fmt::Debug {
    //NOTE: we need the Clone + 'static either way
    type VmHandle: Handle + Send + 'static + Sync;
    type Config: DeserializeOwned + std::fmt::Debug;

    /// The capnp backend config type that parameterizes `VmSpec(BackendConfig)` on the wire.
    type BackendConfig: ::capnp::traits::Owned + 'static;

    /// Domain type extracted from the wire format. Each backend defines how to convert
    /// from `vm_spec::Reader<'a, Self::BackendConfig>` into this type.
    type CreateVmSpec<'a>: TryFrom<
            commands::common_capnp::vm_spec::Reader<'a, Self::BackendConfig>,
            Error = capnp::Error,
        >;

    fn create_id() -> String; //TODO: maybe this should be a tpye of the interface?

    fn create_vm(
        &self,
        source: Self::CreateVmSpec<'_>,
    ) -> impl Future<Output = Result<CreateCommand<Self>, Error>>
    where
        Self: Sized;
}

/// Point-in-time resource sample of one VM. Counters are cumulative; the
/// console derives rates from consecutive samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VmStats {
    uptime_s: u64,
    cpu_ms: u64,
    rss_bytes: u64,
    net_rx: u64,
    net_tx: u64,
    disk_read: u64,
    disk_write: u64,
}

impl VmStats {
    /// Process-level sample (uptime, VMM CPU time, VMM resident memory).
    #[must_use]
    pub fn new(uptime_s: u64, cpu_ms: u64, rss_bytes: u64) -> Self {
        Self {
            uptime_s,
            cpu_ms,
            rss_bytes,
            ..Self::default()
        }
    }

    /// Adds guest device counters (bytes).
    #[must_use]
    pub fn with_counters(
        mut self,
        net_rx: u64,
        net_tx: u64,
        disk_read: u64,
        disk_write: u64,
    ) -> Self {
        self.net_rx = net_rx;
        self.net_tx = net_tx;
        self.disk_read = disk_read;
        self.disk_write = disk_write;
        self
    }

    #[must_use]
    pub fn uptime_s(&self) -> u64 {
        self.uptime_s
    }

    #[must_use]
    pub fn cpu_ms(&self) -> u64 {
        self.cpu_ms
    }

    #[must_use]
    pub fn rss_bytes(&self) -> u64 {
        self.rss_bytes
    }

    #[must_use]
    pub fn net_rx(&self) -> u64 {
        self.net_rx
    }

    #[must_use]
    pub fn net_tx(&self) -> u64 {
        self.net_tx
    }

    #[must_use]
    pub fn disk_read(&self) -> u64 {
        self.disk_read
    }

    #[must_use]
    pub fn disk_write(&self) -> u64 {
        self.disk_write
    }
}

#[derive(Debug)]
pub enum HandleError {
    Start(String),
    Cleanup(String),
    Backup(String),
}

impl std::fmt::Display for HandleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandleError::Start(msg) => write!(f, "start failed: {msg}"),
            HandleError::Cleanup(msg) => write!(f, "cleanup failed: {msg}"),
            HandleError::Backup(msg) => write!(f, "backup failed: {msg}"),
        }
    }
}

impl std::error::Error for HandleError {}

/// This is the interface used to communicate with the the VM itself, either a process in case of cloud hypervisor or whatever else is needed
/// However we might want an id? is good enough to have it in the registry.s map only?
/// A handle that holds the process running the VM and the client to communicate with him.
/// maybe don't needed actually, or yes because the registry needs to hold it and do stuff with it. at least check that everything is running ok, start, stop and other stuff from either the registry or the server
pub trait Handle
where
    Self: Sized,
{
    fn ip(&self) -> &str;

    /// Resource sample written in the worker state lines. Backends without
    /// metrics keep the default (all zeros).
    fn stats(&self) -> impl Future<Output = VmStats> + Send;

    fn start(&self) -> impl Future<Output = Result<(), HandleError>> + Send;

    fn delete(self) -> impl Future<Output = Result<(), HandleError>> + Send;

    fn health(&self) -> impl Future<Output = Result<(), HandleError>> + Send;

    /// Pause a running VM.
    fn pause(&self) -> impl Future<Output = Result<(), HandleError>> + Send;

    /// Resume a previously paused VM.
    fn resume(&self) -> impl Future<Output = Result<(), HandleError>> + Send;

    /// Take a full live snapshot (memory + device state) of the VM into `destination`.
    /// Implementations are expected to leave the VM running (pause+snapshot+resume).
    fn snapshot(
        &self,
        destination: std::path::PathBuf,
    ) -> impl Future<Output = Result<(), HandleError>> + Send;

    /// Take a disk-only backup of the VM into `destination`.
    /// Implementations are expected to briefly pause the VM for consistency, copy the
    /// writable disk file with `tokio::fs::copy`, then resume.
    fn backup_disk(
        &self,
        destination: std::path::PathBuf,
    ) -> impl Future<Output = Result<(), HandleError>> + Send;
}
