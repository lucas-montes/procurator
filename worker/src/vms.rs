use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, Receiver, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use libc::EFD_NONBLOCK;
use serde::{Deserialize, Serialize};

use seccompiler::SeccompAction;
use vmm::api::{ApiAction, VmBoot, VmCreate, VmDelete, VmPause, VmResume, VmShutdown};
use vmm::vm_config::VmConfig;
use vmm::{Error as VmmError, VmmThreadHandle, VmmVersionInfo, start_vmm_thread};
use vmm_sys_util::eventfd::EventFd;

/// Unique identifier for a VM
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct VmId(String);

impl VmId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for VmId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Errors that can occur during VM management
#[derive(Debug)]
pub enum VmManagerError {
    VmAlreadyExists(String),

    VmNotFound(String),

    HypervisorCreation(hypervisor::HypervisorError),

    VmmCreation(VmmError),

    VmStart(String),

    VmStop(String),

    ConfigRead(std::io::Error),

    ConfigParse(serde_json::Error),

    NetworkSetup(String),

    ThreadPanic,

    ApiError(String),
}

/// Configuration for the VM manager
#[derive(Debug, Clone)]
pub struct VmManagerConfig {
    /// How often to poll VMs for metrics (in seconds)
    pub metrics_poll_interval: Duration,

    /// Whether to automatically restart failed VMs
    pub auto_restart: bool,

    /// Base directory for VM artifacts (configs, logs, etc.)
    pub vm_artifacts_dir: PathBuf,

    /// Network bridge name for TAP devices
    pub network_bridge: String,

    /// Base subnet for VM networking (e.g., "10.100.0.0/16")
    pub vm_subnet_base: String,
}

impl Default for VmManagerConfig {
    fn default() -> Self {
        Self {
            metrics_poll_interval: Duration::from_secs(5),
            auto_restart: true,
            vm_artifacts_dir: PathBuf::from("/var/lib/procurator/vms"),
            network_bridge: "br-procurator".to_string(),
            vm_subnet_base: "10.100.0.0/16".to_string(),
        }
    }
}

/// VM status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmStatus {
    Creating,
    Running,
    Paused,
    Stopped,
    Failed(String),
}

/// VM metrics for monitoring and reporting
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VmMetrics {
    pub cpu_usage_percent: f64,
    pub memory_mb: u64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
}

/// Handle to a running VM and its VMM thread
pub struct VmHandle {
    pub id: VmId,
    pub hash: String,
    pub config: VmConfig,
    pub status: Arc<Mutex<VmStatus>>,
    pub metrics: Arc<Mutex<VmMetrics>>,

    /// The VMM thread handle (for joining on shutdown)
    vmm_handle: Option<VmmThreadHandle>,

    /// Channel to send API requests to the VMM
    api_sender: Sender<vmm::api::ApiRequest>,

    /// EventFd to wake up the VMM control loop
    api_event: EventFd,

    /// Network interface name (TAP device)
    tap_device: Option<String>,
}

impl VmHandle {
    /// Send a request to the VMM and wait for response
    fn send_api_request<A: ApiAction>(&self, action: A, data: A::RequestBody) -> Result<A::ResponseBody> {
        action
            .send(
                self.api_event.try_clone().context("Failed to clone API event")?,
                self.api_sender.clone(),
                data,
            )
            .map_err(|e| anyhow!("API request failed: {:?}", e))
    }

    /// Boot the VM
    pub fn boot(&self) -> Result<()> {
        tracing::info!(vm_id = %self.id.as_str(), "Booting VM");
        self.send_api_request(VmBoot, ())?;
        *self.status.lock().unwrap() = VmStatus::Running;
        Ok(())
    }

    /// Pause the VM
    pub fn pause(&self) -> Result<()> {
        tracing::info!(vm_id = %self.id.as_str(), "Pausing VM");
        self.send_api_request(VmPause, ())?;
        *self.status.lock().unwrap() = VmStatus::Paused;
        Ok(())
    }

    /// Resume the VM
    pub fn resume(&self) -> Result<()> {
        tracing::info!(vm_id = %self.id.as_str(), "Resuming VM");
        self.send_api_request(VmResume, ())?;
        *self.status.lock().unwrap() = VmStatus::Running;
        Ok(())
    }

    /// Shutdown the VM gracefully
    pub fn shutdown(&mut self) -> Result<()> {
        tracing::info!(vm_id = %self.id.as_str(), "Shutting down VM");

        // Try graceful shutdown first
        if let Err(e) = self.send_api_request(VmShutdown, ()) {
            tracing::warn!(vm_id = %self.id.as_str(), error = ?e, "Graceful shutdown failed, forcing delete");
            let _ = self.send_api_request(VmDelete, ());
        }

        *self.status.lock().unwrap() = VmStatus::Stopped;

        // Clean up TAP device if it exists
        if let Some(ref tap) = self.tap_device {
            if let Err(e) = cleanup_tap_device(tap) {
                tracing::warn!(tap = tap, error = ?e, "Failed to clean up TAP device");
            }
        }

        // Join the VMM thread if it exists
        if let Some(handle) = self.vmm_handle.take() {
            if let Err(e) = handle.thread_handle.join() {
                tracing::error!(vm_id = %self.id.as_str(), "VMM thread panicked: {:?}", e);
            }
        }

        Ok(())
    }

    /// Get current VM info and update metrics
    pub fn update_metrics(&self) -> Result<()> {
        // TODO: Implement proper metrics collection
        // For now, this is a placeholder - cloud-hypervisor's VmInfo doesn't provide
        // detailed runtime metrics. You'll need to either:
        // 1. Use a guest agent (like qemu-guest-agent equivalent)
        // 2. Parse virtio device statistics
        // 3. Use external monitoring

        tracing::trace!(vm_id = %self.id.as_str(), "Updating VM metrics");
        Ok(())
    }
}

/// Main VM manager that orchestrates multiple VMs
pub struct VmManager {
    config: VmManagerConfig,

    /// Shared hypervisor instance for all VMs
    hypervisor: Arc<dyn hypervisor::Hypervisor>,

    /// Map of VM ID to VM handle
    vms: Arc<Mutex<HashMap<VmId, VmHandle>>>,

    /// Network state tracking
    network_allocator: Arc<Mutex<NetworkAllocator>>,
}

impl VmManager {
    /// Create a new VM manager
    pub fn new(config: VmManagerConfig) -> Result<Self> {
        // Create shared hypervisor instance
        let hypervisor = hypervisor::new().context("Failed to create hypervisor")?;

        tracing::info!(
            hypervisor_type = ?hypervisor.hypervisor_type(),
            "Created hypervisor instance"
        );

        // Ensure artifacts directory exists
        std::fs::create_dir_all(&config.vm_artifacts_dir)
            .context("Failed to create VM artifacts directory")?;

        // Set up network bridge if it doesn't exist
        setup_network_bridge(&config.network_bridge)?;

        Ok(Self {
            config,
            hypervisor: Arc::new(hypervisor),
            vms: Arc::new(Mutex::new(HashMap::new())),
            network_allocator: Arc::new(Mutex::new(NetworkAllocator::new())),
        })
    }

    /// Create and start a new VM from a Nix store path
    pub fn create_vm(
        &self,
        vm_id: VmId,
        hash: String,
        nix_store_path: &Path,
    ) -> Result<()> {
        // Check if VM already exists
        {
            let vms = self.vms.lock().unwrap();
            if vms.contains_key(&vm_id) {
                return Err(VmManagerError::VmAlreadyExists(vm_id.as_str().to_string()).into());
            }
        }

        tracing::info!(
            vm_id = %vm_id.as_str(),
            hash = %hash,
            nix_path = %nix_store_path.display(),
            "Creating new VM"
        );

        // Read VM config from Nix store path
        let config_path = nix_store_path.join("vm-config.json");
        let config = read_vm_config(&config_path)?;

        // Allocate network resources
        let (tap_device, vm_ip) = {
            let mut allocator = self.network_allocator.lock().unwrap();
            allocator.allocate_network(&vm_id, &self.config.network_bridge)?
        };

        tracing::info!(
            vm_id = %vm_id.as_str(),
            tap = %tap_device,
            ip = %vm_ip,
            "Allocated network resources"
        );

        let api_event = EventFd::new(EFD_NONBLOCK)
            .context("Failed to create API event fd")?;

        let exit_event = EventFd::new(EFD_NONBLOCK)
            .context("Failed to create exit event fd")?;

        #[cfg(feature = "guest_debug")]
        let debug_event = EventFd::new(EFD_NONBLOCK)
            .context("Failed to create debug event fd")?;

        #[cfg(feature = "guest_debug")]
        let vm_debug_event = EventFd::new(EFD_NONBLOCK)
            .context("Failed to create VM debug event fd")?;

        let status = Arc::new(Mutex::new(VmStatus::Creating));
        let metrics = Arc::new(Mutex::new(VmMetrics::default()));

        // Create channels for VMM communication
        let (api_sender, api_receiver) = channel();

        let vmm_version = VmmVersionInfo::new(
            env!("CARGO_PKG_VERSION"),
            env!("CARGO_PKG_VERSION"),
        );

        // Start the VMM thread using cloud-hypervisor's public API
        let vmm_handle = start_vmm_thread(
            vmm_version,
            &None,  // http_path
            None,   // http_fd
            #[cfg(feature = "dbus_api")]
            None,   // dbus_options
            api_event.try_clone().context("Failed to clone API event")?,
            api_sender.clone(),
            api_receiver,
            #[cfg(feature = "guest_debug")]
            None,   // debug_path
            #[cfg(feature = "guest_debug")]
            debug_event,
            #[cfg(feature = "guest_debug")]
            vm_debug_event,
            exit_event,
            &SeccompAction::Allow,  // TODO: Configure properly
            self.hypervisor.clone(),
            false,  // landlock_enable
        ).context("Failed to start VMM thread")?;


        let vm_handle = VmHandle {
            id: vm_id.clone(),
            hash,
            config,
            status,
            metrics,
            vmm_handle: Some(vmm_handle),
            api_sender,
            api_event,
            tap_device: Some(tap_device),
        };

        // Create the VM
        vm_handle.send_api_request(VmCreate, Box::new(vm_handle.config.clone()))?;

        // Boot the VM
        vm_handle.boot()?;

        // Store the handle
        {
            let mut vms = self.vms.lock().unwrap();
            vms.insert(vm_id, vm_handle);
        }

        Ok(())
    }

    /// Stop and remove a VM
    pub fn remove_vm(&self, vm_id: &VmId) -> Result<()> {
        tracing::info!(vm_id = %vm_id.as_str(), "Removing VM");

        let mut vm_handle = {
            let mut vms = self.vms.lock().unwrap();
            vms.remove(vm_id)
                .ok_or_else(|| VmManagerError::VmNotFound(vm_id.as_str().to_string()))?
        };

        // Shutdown the VM
        vm_handle.shutdown()?;

        // Free network resources
        {
            let mut allocator = self.network_allocator.lock().unwrap();
            allocator.free_network(vm_id);
        }

        Ok(())
    }

    /// Get status of a specific VM
    pub fn get_vm_status(&self, vm_id: &VmId) -> Result<VmStatus> {
        let vms = self.vms.lock().unwrap();
        let vm = vms.get(vm_id)
            .ok_or_else(|| VmManagerError::VmNotFound(vm_id.as_str().to_string()))?;
        Ok(vm.status.lock().unwrap().clone())
    }

    /// Get metrics for a specific VM
    pub fn get_vm_metrics(&self, vm_id: &VmId) -> Result<VmMetrics> {
        let vms = self.vms.lock().unwrap();
        let vm = vms.get(vm_id)
            .ok_or_else(|| VmManagerError::VmNotFound(vm_id.as_str().to_string()))?;
        vm.update_metrics()?;
        Ok(vm.metrics.lock().unwrap().clone())
    }

    /// Get all VM IDs
    pub fn list_vms(&self) -> Vec<VmId> {
        let vms = self.vms.lock().unwrap();
        vms.keys().cloned().collect()
    }

    /// Start metrics polling background task
    pub fn start_metrics_polling(&self) -> JoinHandle<()> {
        let vms = self.vms.clone();
        let interval = self.config.metrics_poll_interval;

        thread::Builder::new()
            .name("metrics-poller".to_string())
            .spawn(move || {
                loop {
                    thread::sleep(interval);

                    let vms = vms.lock().unwrap();
                    for (vm_id, vm_handle) in vms.iter() {
                        if let Err(e) = vm_handle.update_metrics() {
                            tracing::warn!(
                                vm_id = %vm_id.as_str(),
                                error = ?e,
                                "Failed to update VM metrics"
                            );
                        }
                    }
                }
            })
            .expect("Failed to spawn metrics polling thread")
    }
}

/// Read VM config from a JSON file
fn read_vm_config(path: &Path) -> Result<VmConfig> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config from {}", path.display()))?;

    let config: VmConfig = serde_json::from_str(&contents)
        .with_context(|| format!("Failed to parse config from {}", path.display()))?;

    Ok(config)
}

/// Network resource allocator
struct NetworkAllocator {
    allocated_ips: HashMap<VmId, String>,
    next_ip_index: u32,
}

impl NetworkAllocator {
    fn new() -> Self {
        Self {
            allocated_ips: HashMap::new(),
            next_ip_index: 2, // Start from .2 (.1 is usually gateway)
        }
    }

    fn allocate_network(&mut self, vm_id: &VmId, bridge: &str) -> Result<(String, String)> {
        // Generate TAP device name
        let tap_name = format!("tap-{}", &vm_id.as_str()[..8.min(vm_id.as_str().len())]);

        // Allocate IP from subnet
        let ip = format!("10.100.0.{}", self.next_ip_index);
        self.next_ip_index += 1;

        // Create TAP device
        create_tap_device(&tap_name, bridge)?;

        self.allocated_ips.insert(vm_id.clone(), ip.clone());

        Ok((tap_name, ip))
    }

    fn free_network(&mut self, vm_id: &VmId) {
        self.allocated_ips.remove(vm_id);
    }
}

/// Set up network bridge for VMs
fn setup_network_bridge(bridge_name: &str) -> Result<()> {
    use std::process::Command;

    // Check if bridge exists
    let output = Command::new("ip")
        .args(&["link", "show", bridge_name])
        .output();

    if output.is_err() || !output.unwrap().status.success() {
        tracing::info!(bridge = bridge_name, "Creating network bridge");

        // Create bridge
        Command::new("ip")
            .args(&["link", "add", "name", bridge_name, "type", "bridge"])
            .status()
            .context("Failed to create bridge")?;

        // Set bridge up
        Command::new("ip")
            .args(&["link", "set", bridge_name, "up"])
            .status()
            .context("Failed to bring bridge up")?;

        // Assign IP to bridge (gateway)
        Command::new("ip")
            .args(&["addr", "add", "10.100.0.1/16", "dev", bridge_name])
            .status()
            .context("Failed to assign IP to bridge")?;
    }

    Ok(())
}

/// Create a TAP device and attach it to a bridge
fn create_tap_device(tap_name: &str, bridge: &str) -> Result<()> {
    use std::process::Command;

    tracing::info!(tap = tap_name, bridge = bridge, "Creating TAP device");

    // Create TAP device
    Command::new("ip")
        .args(&["tuntap", "add", "dev", tap_name, "mode", "tap"])
        .status()
        .context("Failed to create TAP device")?;

    // Set TAP device up
    Command::new("ip")
        .args(&["link", "set", tap_name, "up"])
        .status()
        .context("Failed to bring TAP device up")?;

    // Attach to bridge
    Command::new("ip")
        .args(&["link", "set", tap_name, "master", bridge])
        .status()
        .context("Failed to attach TAP to bridge")?;

    Ok(())
}

/// Clean up a TAP device
fn cleanup_tap_device(tap_name: &str) -> Result<()> {
    use std::process::Command;

    tracing::info!(tap = tap_name, "Cleaning up TAP device");

    Command::new("ip")
        .args(&["link", "delete", tap_name])
        .status()
        .context("Failed to delete TAP device")?;

    Ok(())
}
