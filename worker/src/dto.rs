//! Message types for communication between Server (RPC adapter) and Node (VM owner).
//!
//! The Server sends VmCommands through the channel. The Node processes them
//! and replies via the embedded oneshot sender. No capnp types cross the channel —
//! only plain Rust structs.

use std::fmt;

use tokio::sync::{mpsc, oneshot};

// ─── Error type that crosses the channel ───────────────────────────────────

/// Errors returned by Node/VmManager through the oneshot reply.
/// Converted to `capnp::Error` at the RPC boundary in Server.
#[derive(Debug)]
pub enum VmError {
    /// The requested VM does not exist in the manager's table
    NotFound(String),
    /// A VM with that ID already exists
    AlreadyExists(String),
    /// The CloudHypervisor REST call failed
    Hypervisor(String),
    /// The CH process failed to spawn or died unexpectedly
    ProcessFailed(String),
    /// The command channel is closed (Node is down)
    ManagerDown,
    /// Catch-all for unexpected failures
    Internal(String),
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmError::NotFound(id) => write!(f, "VM not found: {id}"),
            VmError::AlreadyExists(id) => write!(f, "VM already exists: {id}"),
            VmError::Hypervisor(msg) => write!(f, "cloud-hypervisor error: {msg}"),
            VmError::ProcessFailed(msg) => write!(f, "process error: {msg}"),
            VmError::ManagerDown => write!(f, "VM manager is down"),
            VmError::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for VmError {}

impl From<VmError> for capnp::Error {
    fn from(e: VmError) -> Self {
        capnp::Error::failed(e.to_string())
    }
}

// ─── Internal VM data types (no capnp, no CH specifics) ───────────────────

/// Internal representation of a VM's desired configuration.
/// Built from capnp VmSpec in the Server, consumed by Node/VmManager.
#[derive(Debug, Clone)]
pub struct VmSpec {
    id: String,
    name: String,
    store_path: String,
    content_hash: String,
    cpu: f32,
    memory_bytes: u64,
    kernel_path: String,
    initrd_path: Option<String>,
    disk_image_path: String,
    cmdline: Option<String>,
    network_allowed_domains: Vec<String>,
}

impl VmSpec {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        name: String,
        store_path: String,
        content_hash: String,
        cpu: f32,
        memory_bytes: u64,
        kernel_path: String,
        initrd_path: Option<String>,
        disk_image_path: String,
        cmdline: Option<String>,
        network_allowed_domains: Vec<String>,
    ) -> Self {
        Self {
            id,
            name,
            store_path,
            content_hash,
            cpu,
            memory_bytes,
            kernel_path,
            initrd_path,
            disk_image_path,
            cmdline,
            network_allowed_domains,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn store_path(&self) -> &str {
        &self.store_path
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn cpu(&self) -> f32 {
        self.cpu
    }

    pub fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }

    pub fn kernel_path(&self) -> &str {
        &self.kernel_path
    }

    pub fn initrd_path(&self) -> Option<&str> {
        self.initrd_path.as_deref()
    }

    pub fn disk_image_path(&self) -> &str {
        &self.disk_image_path
    }

    pub fn cmdline(&self) -> Option<&str> {
        self.cmdline.as_deref()
    }

    pub fn network_allowed_domains(&self) -> &[String] {
        &self.network_allowed_domains
    }
}

/// Internal representation of a VM's observed status.
/// Built by Node/VmManager, consumed by Server to fill capnp responses.
#[derive(Debug, Clone)]
pub struct VmInfo {
    id: String,
    worker_id: String,
    status: VmStatus,
    desired_hash: String,
    observed_hash: String,
    metrics: VmMetrics,
}

impl VmInfo {
    pub fn new(
        id: String,
        worker_id: String,
        status: VmStatus,
        desired_hash: String,
        observed_hash: String,
        metrics: VmMetrics,
    ) -> Self {
        Self {
            id,
            worker_id,
            status,
            desired_hash,
            observed_hash,
            metrics,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    pub fn status(&self) -> &VmStatus {
        &self.status
    }

    pub fn desired_hash(&self) -> &str {
        &self.desired_hash
    }

    pub fn observed_hash(&self) -> &str {
        &self.observed_hash
    }

    pub fn metrics(&self) -> &VmMetrics {
        &self.metrics
    }
}

#[derive(Debug, Clone)]
pub enum VmStatus {
    Creating,
    Running,
    Paused,
    Stopping,
    Stopped,
    Failed(String),
}

impl VmStatus {
    pub fn as_str(&self) -> &str {
        match self {
            VmStatus::Creating => "creating",
            VmStatus::Running => "running",
            VmStatus::Paused => "paused",
            VmStatus::Stopping => "stopping",
            VmStatus::Stopped => "stopped",
            VmStatus::Failed(_) => "failed",
        }
    }

    pub fn is_drifted(&self, desired: &str, observed: &str) -> bool {
        desired != observed
    }
}

#[derive(Debug, Clone, Default)]
pub struct VmMetrics {
    pub cpu_usage: f32,
    pub memory_usage: u64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
}

/// Worker-level status info.
#[derive(Debug, Clone)]
pub struct WorkerInfo {
    id: String,
    healthy: bool,
    generation: u64,
    running_vms: u32,
}

impl WorkerInfo {
    pub fn new(id: String, healthy: bool, generation: u64, running_vms: u32) -> Self {
        Self {
            id,
            healthy,
            generation,
            running_vms,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn healthy(&self) -> bool {
        self.healthy
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn running_vms(&self) -> u32 {
        self.running_vms
    }
}



// ─── Commands sent from Server → Node ──────────────────────────────────────

/// Commands (payload) sent from Server → Node.
/// Plain Rust types only — no capnp errors here.
#[derive(Debug)]
pub enum CommandPayload {
    Create(VmSpec),
    Delete(String),
    List,
    GetWorkerStatus,
}

/// Unified response envelope for commands. The Node replies with this
/// over the oneshot channel embedded in `Message`.
#[derive(Debug)]
pub enum CommandResponse {
    Unit,
    VmList(Vec<VmInfo>),
    WorkerInfo(WorkerInfo),
}

/// Message sent over the mpsc channel. Contains the command payload
/// and a oneshot `reply` sender used by the Node to respond.
pub struct Message {
    data: CommandPayload,
    reply: oneshot::Sender<Result<CommandResponse, VmError>>,
}

impl Message {
    pub fn into_parts(self) -> (CommandPayload, oneshot::Sender<Result<CommandResponse, VmError>>) {
        (self.data, self.reply)
    }
}

// ─── Channel wrapper (cloneable handle for Server) ─────────────────────────

/// Thin wrapper around `mpsc::Sender<Message>` used by the Server.
///
/// The oneshot channel is created internally — the caller just passes
/// a `CommandPayload` and awaits a `Result<CommandResponse, VmError>`.
#[derive(Clone)]
pub struct CommandSender(mpsc::Sender<Message>);

impl CommandSender {
    pub fn new(tx: mpsc::Sender<Message>) -> Self {
        Self(tx)
    }

    /// Send a command to the Node and await the response.
    ///
    /// Creates the oneshot channel, wraps the payload in a `Message`,
    /// sends it, and awaits the reply — all in one call.
    pub async fn request(&self, data: CommandPayload) -> Result<CommandResponse, VmError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let msg = Message { data, reply: reply_tx };
        self.0.send(msg).await.map_err(|_| VmError::ManagerDown)?;
        reply_rx.await.map_err(|_| VmError::ManagerDown)?
    }
}

impl From<mpsc::Sender<Message>> for CommandSender {
    fn from(tx: mpsc::Sender<Message>) -> Self {
        Self(tx)
    }
}
