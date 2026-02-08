mod commands_capnp {
    include!(concat!(env!("OUT_DIR"), "/commands_capnp.rs"));
}

pub use commands_capnp::*;

// ============================================================================
// Nix Eval Server → Master: Publish desired state
// ============================================================================

#[derive(Debug, Clone)]
pub struct PublishDesiredStateRequest {
    pub commit: String,
    pub generation: u64,
    pub intent_hash: String,
    pub vm_specs: Vec<VmSpecRequest>,
}

#[derive(Debug, Clone)]
pub struct VmSpecRequest {
    pub id: String,
    pub name: String,
    pub store_path: String,
    pub content_hash: String,
    pub cpu: f32,
    pub memory_bytes: u64,
    pub labels: Vec<(String, String)>,
    pub replicas: u32,
    pub network_allowed_domains: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum PublishResult {
    Ok,
    Err(String),
}

// ============================================================================
// Master ← Worker: Assignment and status (pull + push)
// ============================================================================

#[derive(Debug, Clone)]
pub struct GetAssignmentRequest {
    pub worker_id: String,
    pub last_seen_generation: u64,
}

#[derive(Debug, Clone)]
pub enum AssignmentResult {
    Ok(Assignment),
    Err(String),
}

#[derive(Debug, Clone)]
pub struct Assignment {
    pub generation: u64,
    pub desired_vms: Vec<VmSpecRequest>,
}

#[derive(Debug, Clone)]
pub struct PushObservedStateRequest {
    pub worker_id: String,
    pub generation: u64,
    pub running_vms: Vec<RunningVmObserved>,
    pub metrics: WorkerMetricsObserved,
}

#[derive(Debug, Clone)]
pub enum StatusResult {
    Ok,
    Err(String),
}

#[derive(Debug, Clone)]
pub struct RunningVmObserved {
    pub id: String,
    pub content_hash: String,
    pub status: String,  // "running", "stopping", "failed"
    pub uptime_seconds: u64,
    pub metrics: VmMetricsObserved,
}

#[derive(Debug, Clone)]
pub struct VmMetricsObserved {
    pub cpu_usage: f32,
    pub memory_usage_bytes: u64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct WorkerMetricsObserved {
    pub available_cpu: f32,
    pub available_memory_bytes: u64,
    pub disk_usage_bytes: u64,
    pub uptime_seconds: u64,
}

// ============================================================================
// Master → User CLI: Status queries + VM interaction
// ============================================================================

#[derive(Debug, Clone)]
pub enum ClusterStatusResult {
    Ok(ClusterStatusResponse),
    Err(String),
}

#[derive(Debug, Clone)]
pub struct ClusterStatusResponse {
    pub active_generation: u64,
    pub active_commit: String,
    pub convergence_percent: u32,
    pub workers: Vec<WorkerStatusResponse>,
    pub vms: Vec<VmStatusResponse>,
}

#[derive(Debug, Clone)]
pub enum WorkerStatusResult {
    Ok(WorkerStatusResponse),
    Err(String),
}

#[derive(Debug, Clone)]
pub struct WorkerStatusResponse {
    pub id: String,
    pub healthy: bool,
    pub generation: u64,
    pub running_vms_count: u32,
    pub available_resources: Resources,
    pub metrics: WorkerMetricsObserved,
}

#[derive(Debug, Clone)]
pub enum VmStatusResult {
    Ok(VmStatusResponse),
    Err(String),
}

#[derive(Debug, Clone)]
pub struct VmStatusResponse {
    pub id: String,
    pub worker_id: String,
    pub desired_hash: String,
    pub observed_hash: String,
    pub status: String,  // "pending", "running", "drifted", "failed"
    pub drifted: bool,
    pub metrics: VmMetricsObserved,
}

#[derive(Debug, Clone)]
pub enum GenerationHistoryResult {
    Ok(Vec<GenerationResponse>),
    Err(String),
}

#[derive(Debug, Clone)]
pub struct GenerationResponse {
    pub number: u64,
    pub commit: String,
    pub intent_hash: String,
    pub timestamp_unix: u64,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct Resources {
    pub cpu: f32,
    pub memory_bytes: u64,
}

// ============================================================================
// VM Interaction/Inspection
// ============================================================================

#[derive(Debug, Clone)]
pub struct GetVmLogsRequest {
    pub vm_id: String,
    pub follow: bool,
    pub tail_lines: u32,
}

#[derive(Debug, Clone)]
pub enum VmLogsResult {
    Ok(VmLogs),
    Err(String),
}

#[derive(Debug, Clone)]
pub struct VmLogs {
    pub logs: String,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct ExecInVmRequest {
    pub vm_id: String,
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ExecResult {
    Ok(ExecOutput),
    Err(String),
}

#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone)]
pub enum ConnectionInfoResult {
    Ok(ConnectionInfo),
    Err(String),
}

#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub vm_id: String,
    pub worker_host: String,
    pub ssh_port: u16,
    pub console_port: u16,
    pub username: String,
}

// ============================================================================
// Internal Master Events (channel-based)
// ============================================================================
// These are the internal Rust events passed between Master components
// via mpsc channels. Not directly Cap'n Proto.

#[derive(Debug)]
pub enum MasterEvent {
    PublishDesiredState {
        req: PublishDesiredStateRequest,
        reply: tokio::sync::oneshot::Sender<PublishResult>,
    },
    GetClusterStatus {
        reply: tokio::sync::oneshot::Sender<ClusterStatusResult>,
    },
    GetWorkerStatus {
        worker_id: String,
        reply: tokio::sync::oneshot::Sender<WorkerStatusResult>,
    },
    GetVmStatus {
        vm_id: String,
        reply: tokio::sync::oneshot::Sender<VmStatusResult>,
    },
    GetGenerationHistory {
        reply: tokio::sync::oneshot::Sender<GenerationHistoryResult>,
    },
    GetVmLogs {
        req: GetVmLogsRequest,
        reply: tokio::sync::oneshot::Sender<VmLogsResult>,
    },
    ExecInVm {
        req: ExecInVmRequest,
        reply: tokio::sync::oneshot::Sender<ExecResult>,
    },
    GetVmConnectionInfo {
        vm_id: String,
        reply: tokio::sync::oneshot::Sender<ConnectionInfoResult>,
    },
}

#[derive(Debug)]
pub enum WorkerEvent {
    GetAssignment {
        req: GetAssignmentRequest,
        reply: tokio::sync::oneshot::Sender<AssignmentResult>,
    },
    PushObservedState {
        req: PushObservedStateRequest,
        reply: tokio::sync::oneshot::Sender<StatusResult>,
    },
}
