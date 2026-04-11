
/// Internal representation of a VM's desired configuration.
/// Built from capnp VmSpec in the Server, consumed by Node/VmManager.
/// Also deserializable from the JSON produced by the Nix `vmSpecJson` output.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmSpec {
    toplevel: String,
    kernel_path: String,
    initrd_path: String,
    disk_image_path: String,
    cmdline: String,
    cpu: u32,
    memory_mb: u32,
    network_allowed_domains: Vec<String>,
}

impl VmSpec {
    pub fn new(
        toplevel: String,
        kernel_path: String,
        initrd_path: String,
        disk_image_path: String,
        cmdline: String,
        cpu: u32,
        memory_mb: u32,
        network_allowed_domains: Vec<String>,
    ) -> Self {
        Self {
            toplevel,
            kernel_path,
            initrd_path,
            disk_image_path,
            cmdline,
            cpu,
            memory_mb,
            network_allowed_domains,
        }
    }

    pub fn toplevel(&self) -> &str {
        &self.toplevel
    }

    pub fn kernel_path(&self) -> &str {
        &self.kernel_path
    }

    pub fn initrd_path(&self) -> &str {
        &self.initrd_path
    }

    pub fn disk_image_path(&self) -> &str {
        &self.disk_image_path
    }

    pub fn cmdline(&self) -> &str {
        &self.cmdline
    }

    pub fn cpu(&self) -> u32 {
        self.cpu
    }

    pub fn memory_mb(&self) -> u32 {
        self.memory_mb
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
    Running,
}

impl VmStatus {
    pub fn as_str(&self) -> &str {
        match self {
            VmStatus::Running => "running",
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
