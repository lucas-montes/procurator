//! Cloud Hypervisor VMM backend implementation.
//!
//! Three types work together:
//!
//! - [`CloudHypervisor`] — per-VM REST client (implements [`Vmm`]).
//! - [`ChProcess`] — handle to one `cloud-hypervisor` OS process (implements [`VmmProcess`]).
//! - [`CloudHypervisorBackend`] — factory that spawns CH processes (implements [`VmmBackend`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use hyperlocal::{UnixClientExt, Uri as UnixUri};
use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};
use tracing::{debug, info, warn};
use futures::stream::TryStreamExt;
use rtnetlink;

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
    OperationFailed(String),
    Serialization(serde_json::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Communication(msg) => write!(f, "Communication error: {}", msg),
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
    type Error = Error;

    async fn create(&self, config: Self::Config) -> Result<(), Self::Error> {
        let body = serde_json::to_string(&config)?;
        debug!(config_json = %body, "vm.create request");

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

        let status = resp.status();
        if !status.is_success() {
            let body_bytes = hyper::body::to_bytes(resp.into_body())
                .await
                .map_err(|e| Error::Communication(e.to_string()))?;
            let error_msg = String::from_utf8_lossy(&body_bytes);
            warn!(http_status = %status, error = %error_msg, "vm.create failed");
            return Err(Error::OperationFailed(format!(
                "Failed to create VM: {}",
                error_msg
            )));
        }

        info!(http_status = %status, "vm.create succeeded");
        Ok(())
    }

    async fn boot(&self) -> Result<(), Self::Error> {
        debug!("vm.boot request");
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

        let status = resp.status();
        let body_bytes = hyper::body::to_bytes(resp.into_body())
            .await
            .map_err(|e| Error::Communication(e.to_string()))?;
        let body_str = String::from_utf8_lossy(&body_bytes);

        if !status.is_success() {
            warn!(http_status = %status, body = %body_str, "vm.boot failed");
            return Err(Error::OperationFailed(format!(
                "Failed to boot VM: {}",
                body_str
            )));
        }

        info!(http_status = %status, body = %body_str, "vm.boot succeeded");
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

// ─── Process handle ───────────────────────────────────────────────────────

/// Handle to one `cloud-hypervisor` OS process.
///
/// Owns the [`Child`], the socket path, and the per-VM working directory.
/// Cleans up all three on [`VmmProcess::cleanup`].
pub struct ChProcess {
    child: Child,
    socket_path: PathBuf,
    /// Per-VM working directory (contains writable disk copy, serial log, etc.)
    vm_dir: PathBuf,
    /// TAP device name owned by this VM. Deleted on cleanup via netlink.
    /// `None` when the VM was started without networking.
    tap_name: Option<String>,
}

impl VmmProcess for ChProcess {
    async fn kill(&mut self) -> Result<(), VmError> {
        self.child
            .kill()
            .await
            .map_err(|e| VmError::ProcessFailed(format!("Failed to kill CH process: {e}")))
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, VmError> {
        self.child
            .try_wait()
            .map_err(|e| VmError::ProcessFailed(format!("Failed to check CH process: {e}")))
    }

    async fn cleanup(&mut self) -> Result<(), VmError> {
        // Log CH output for post-mortem debugging before cleaning up.
        let ch_log = self.vm_dir.join("cloud-hypervisor.log");
        if ch_log.exists() {
            match tokio::fs::read_to_string(&ch_log).await {
                Ok(contents) if !contents.is_empty() => {
                    warn!(
                        path = %ch_log.display(),
                        "cloud-hypervisor log output:\n{}",
                        contents
                    );
                }
                Ok(_) => {
                    debug!("cloud-hypervisor log was empty");
                }
                Err(e) => {
                    warn!(error = %e, "Failed to read cloud-hypervisor log");
                }
            }
        }

        // Delete the TAP device via netlink (best-effort).
        // The worker already has CAP_NET_ADMIN so this works without root.
        if let Some(ref tap) = self.tap_name {
            match delete_tap_device(tap).await {
                Ok(()) => info!(tap = %tap, "TAP device deleted"),
                Err(e) => warn!(tap = %tap, error = %e, "Failed to delete TAP device"),
            }
        }

        if self.socket_path.exists() {
            let _ = tokio::fs::remove_file(&self.socket_path).await;
        }
        // Remove the entire per-VM working directory (writable disk, serial log, etc.)
        if self.vm_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&self.vm_dir).await;
        }
        Ok(())
    }
}

/// Delete a TAP device by name via netlink.
///
/// Requires `CAP_NET_ADMIN` — the worker process holds this via
/// systemd `AmbientCapabilities`.
async fn delete_tap_device(tap_name: &str) -> Result<(), VmError> {
    let (connection, handle, _) = rtnetlink::new_connection()
        .map_err(|e| VmError::Internal(format!("netlink connection failed: {e}")))?;
    tokio::spawn(connection);

    let mut links = handle
        .link()
        .get()
        .match_name(tap_name.to_string())
        .execute();
    let msg = links
        .try_next()
        .await
        .map_err(|e| VmError::Internal(format!("netlink get {tap_name} failed: {e}")))?;

    if let Some(link) = msg {
        handle
            .link()
            .del(link.header.index)
            .execute()
            .await
            .map_err(|e| VmError::Internal(format!("netlink del {tap_name} failed: {e}")))?;
    }
    Ok(())
}

/// Create a TAP device by name via `ioctl` on `/dev/net/tun`.
///
/// TAP devices are created through the TUN/TAP kernel interface, not via
/// netlink. The process:
///   1. `open("/dev/net/tun")`  — requires rw access (netdev group + DeviceAllow)
///   2. `ioctl(fd, TUNSETIFF, &ifreq)` — requires `CAP_NET_ADMIN`
///   3. `ioctl(fd, TUNSETPERSIST, 1)` — makes the TAP survive fd close
///
/// After creation, we use netlink to bring the interface up.
///
/// If the TAP already exists (e.g. from a previous crashed VM), it is
/// deleted first to avoid stale state.
async fn create_tap_device(tap_name: &str) -> Result<(), VmError> {
    // Delete stale TAP if it exists (crash recovery).
    // Best-effort — ignore errors if it doesn't exist.
    let _ = delete_tap_device(tap_name).await;

    // Create TAP via ioctl on /dev/net/tun.
    // This is a blocking syscall so we run it on the blocking pool.
    let name = tap_name.to_string();
    tokio::task::spawn_blocking(move || create_tap_ioctl(&name))
        .await
        .map_err(|e| VmError::Internal(format!("spawn_blocking for TAP creation panicked: {e}")))?
        .map_err(|e| VmError::Internal(format!("TAP ioctl creation failed: {e}")))?;

    // Bring the TAP up via netlink.
    let (connection, handle, _) = rtnetlink::new_connection()
        .map_err(|e| VmError::Internal(format!("netlink connection failed: {e}")))?;
    tokio::spawn(connection);

    let mut links = handle
        .link()
        .get()
        .match_name(tap_name.to_string())
        .execute();
    let msg = links
        .try_next()
        .await
        .map_err(|e| VmError::Internal(format!("netlink get {tap_name} after create: {e}")))?
        .ok_or_else(|| VmError::Internal(format!("TAP {tap_name} not found after creation")))?;

    handle
        .link()
        .set(msg.header.index)
        .up()
        .execute()
        .await
        .map_err(|e| VmError::Internal(format!("netlink set {tap_name} up failed: {e}")))?;

    info!(tap = %tap_name, "TAP device created and brought up");
    Ok(())
}

/// Low-level TAP creation via `ioctl(2)`.
///
/// Opens `/dev/net/tun`, issues `TUNSETIFF` with `IFF_TAP | IFF_NO_PI`,
/// then `TUNSETPERSIST` so the device survives the fd being closed.
/// The fd is then dropped — CH will re-open the persistent TAP by name.
fn create_tap_ioctl(tap_name: &str) -> Result<(), std::io::Error> {
    use std::fs::OpenOptions;
    use std::os::unix::io::AsRawFd;

    // ioctl constants from <linux/if_tun.h>
    const TUNSETIFF: libc::c_ulong = 0x400454ca;
    const TUNSETPERSIST: libc::c_ulong = 0x400454cb;
    const IFF_TAP: libc::c_short = 0x0002;
    const IFF_NO_PI: libc::c_short = 0x1000;

    let tun_fd = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/net/tun")?;

    // Build ifreq struct — name + flags
    let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
    let name_bytes = tap_name.as_bytes();
    if name_bytes.len() >= libc::IFNAMSIZ {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("TAP name too long: {} (max {})", tap_name, libc::IFNAMSIZ - 1),
        ));
    }
    // Copy name into ifr_name (null-terminated)
    unsafe {
        std::ptr::copy_nonoverlapping(
            name_bytes.as_ptr(),
            ifr.ifr_name.as_mut_ptr().cast::<u8>(),
            name_bytes.len(),
        );
    }
    ifr.ifr_ifru.ifru_flags = IFF_TAP | IFF_NO_PI;

    // TUNSETIFF — create the TAP device
    let ret = unsafe { libc::ioctl(tun_fd.as_raw_fd(), TUNSETIFF, &ifr) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // TUNSETPERSIST — keep the TAP alive after we close the fd.
    // CH will re-open it by name when it starts.
    let ret = unsafe { libc::ioctl(tun_fd.as_raw_fd(), TUNSETPERSIST, 1_i32) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // fd is dropped here — the persistent TAP remains in the kernel.
    Ok(())
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
    /// Name of the host bridge to attach VM TAP devices to (e.g. `chbr0`).
    /// Set to `None` to skip TAP-to-bridge attachment (VMs get no network).
    pub bridge_name: Option<String>,
}

impl Default for CloudHypervisorConfig {
    fn default() -> Self {
        Self {
            socket_dir: PathBuf::from("/tmp/procurator/vms"),
            ch_binary: PathBuf::from("cloud-hypervisor"),
            socket_timeout: Duration::from_secs(5),
            bridge_name: Some("chbr0".to_string()),
        }
    }
}

/// Per-VM state created by `prepare()` and consumed by `build_config()` and `spawn()`.
///
/// Tracks the writable paths that replace the immutable Nix store paths.
struct PreparedVm {
    /// Writable copy of the disk image (the Nix store original is read-only)
    writable_disk_path: PathBuf,
    /// Path where CH will write serial console output
    serial_log_path: PathBuf,
    /// Per-VM working directory (parent of disk + serial log)
    vm_dir: PathBuf,
    /// TAP device name for this VM's virtio-net interface.
    /// CH creates the TAP at `vm.create()` time; we attach it to the
    /// bridge between `create()` and `boot()`.
    tap_name: String,
    /// Whether the host bridge exists and networking can be set up.
    /// When `false`, CH is started without `--net` and TAP attachment is skipped.
    /// This allows dev/testing without the NixOS host module.
    network_available: bool,
}

/// Factory that spawns `cloud-hypervisor` processes and creates
/// [`CloudHypervisor`] REST clients.
///
/// This is the production implementation of [`VmmBackend`].
///
/// Tracks per-VM prepared state (writable disk paths, serial log paths)
/// between the `prepare()` and `build_config()`/`spawn()` calls.
/// Uses a `Mutex<HashMap>` for interior mutability since the trait
/// methods take `&self`. The lock is held only briefly for insert/remove.
pub struct CloudHypervisorBackend {
    config: CloudHypervisorConfig,
    /// Per-VM prepared state, keyed by vm_id.
    /// Populated by `prepare()`, consumed by `build_config()` and `spawn()`.
    prepared: Mutex<HashMap<String, PreparedVm>>,
}

impl CloudHypervisorBackend {
    pub fn new(config: CloudHypervisorConfig) -> Self {
        Self {
            config,
            prepared: Mutex::new(HashMap::new()),
        }
    }

    /// Attach the VM's TAP device to the host bridge.
    ///
    /// Called between `vm.create()` (CH creates the TAP) and `vm.boot()`.
    /// The worker process itself already has `CAP_NET_ADMIN`.
    /// Instead of spawning `ip(8)` we talk to the kernel directly via
    /// netlink. doing so avoids the capability‑inheritance problem where a
    /// child process loses the parent's caps and `ip` would fail with
    /// "Operation not permitted".
    pub async fn attach_tap_to_bridge(&self, vm_id: &str) -> Result<(), VmError> {
        let bridge = match &self.config.bridge_name {
            Some(b) => b,
            None => return Ok(()), // No bridge configured — skip
        };

        let (tap_name, network_available) = {
            let guard = self.prepared.lock().expect("prepared lock poisoned");
            let p = guard.get(vm_id).ok_or_else(|| VmError::Internal(format!(
                "No prepared state for VM {vm_id} — cannot find TAP name"
            )))?;
            (p.tap_name.clone(), p.network_available)
        };

        // Bridge didn't exist at prepare() time — nothing to attach.
        if !network_available {
            return Ok(());
        }

        info!(
            vm_id = %vm_id,
            tap = %tap_name,
            bridge = %bridge,
            "Attaching TAP to bridge"
        );

        // We speak netlink directly so we can control the retry behaviour
        // when the interface hasn't appeared yet.  The `rtnetlink` crate
        // returns the link index for a given name, which we then use to set
        // the master/`up` flags.
        let (connection, handle, _) = rtnetlink::new_connection()
            .map_err(|e| VmError::Internal(format!("netlink connection failed: {e}")))?;
        // drive the connection in the background
        tokio::spawn(connection);

        // helper that returns the link index or None if not found
        async fn link_index(
            handle: &rtnetlink::Handle,
            name: &str,
        ) -> Result<Option<u32>, VmError> {
            // `match_name` is a convenience filter provided by rtnetlink that
            // adds the appropriate netlink attribute.  `execute()` returns a
            // `TryStream` of `LinkMessage` objects, so we can call
            // `try_next()` to grab the first (and only) result.
            let mut links = handle.link().get().match_name(name.to_string()).execute();
            let opt_msg = links
                .try_next()
                .await
                .map_err(|e| VmError::Internal(format!("netlink get failed: {e}")))?;
            Ok(opt_msg.map(|m| m.header.index))
        }

        let max_attempts = 20;
        for attempt in 1..=max_attempts {
            match link_index(&handle, &tap_name).await? {
                Some(tap_idx) => {
                    // bridge is expected to exist; if it does not we abort.
                    let bridge_idx = match link_index(&handle, bridge).await? {
                        Some(idx) => idx,
                        None => {
                            return Err(VmError::Internal(format!(
                                "bridge {} not found when attaching TAP",
                                bridge
                            )));
                        }
                    };

                    let attach_res = handle
                        .link()
                        .set(tap_idx)
                        .master(bridge_idx)
                        .up()
                        .execute()
                        .await;
                    match attach_res {
                        Ok(()) => {
                            info!(
                                vm_id = %vm_id,
                                tap = %tap_name,
                                bridge = %bridge,
                                attempts = attempt,
                                "TAP attached to bridge"
                            );
                            return Ok(());
                        }
                        Err(e) => {
                            let stderr = format!("{e}");
                            warn!(
                                vm_id = %vm_id,
                                tap = %tap_name,
                                bridge = %bridge,
                                attempts = attempt,
                                stderr = %stderr,
                                "Failed to attach TAP to bridge — VM may have no network"
                            );
                            return Ok(());
                        }
                    }
                }
                None if attempt < max_attempts => {
                    debug!(
                        vm_id = %vm_id,
                        tap = %tap_name,
                        bridge = %bridge,
                        attempts = attempt,
                        "TAP not visible yet; retrying bridge attach"
                    );
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                None => {
                    warn!(
                        vm_id = %vm_id,
                        tap = %tap_name,
                        bridge = %bridge,
                        "TAP still missing after retries — VM may have no network"
                    );
                    return Ok(());
                }
            }
        }

        warn!(
            vm_id = %vm_id,
            tap = %tap_name,
            bridge = %bridge,
            "Failed to attach TAP to bridge after retries — VM may have no network"
        );
        Ok(())
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

    async fn prepare(&self, vm_id: &str, spec: &VmSpec) -> Result<(), VmError> {
        // 1. Validate that all Nix store paths exist locally
        for (label, path) in [
            ("kernel", spec.kernel_path()),
            ("initrd", spec.initrd_path()),
            ("disk image", spec.disk_image_path()),
        ] {
            if !Path::new(path).exists() {
                return Err(VmError::Internal(format!(
                    "Artifact not found: {label} at {path}. \
                     Ensure the closure has been built or copied to this host."
                )));
            }
        }

        // 2. Create per-VM working directory
        let vm_dir = self.config.socket_dir.join(vm_id);
        tokio::fs::create_dir_all(&vm_dir)
            .await
            .map_err(|e| VmError::ProcessFailed(format!(
                "Failed to create VM directory {}: {e}", vm_dir.display()
            )))?;

        // 3. Copy disk image to a writable location
        //    The Nix store is read-only — CH needs to write to the disk.
        //    tokio::fs::copy uses copy_file_range on Linux (efficient, works on all FS).
        let writable_disk_path = vm_dir.join("disk.img");
        let src = spec.disk_image_path();
        tracing::info!(
            vm_id = %vm_id,
            src = %src,
            dst = %writable_disk_path.display(),
            "Copying disk image to writable location"
        );
        tokio::fs::copy(src, &writable_disk_path)
            .await
            .map_err(|e| VmError::Internal(format!(
                "Failed to copy disk image from {} to {}: {e}",
                src, writable_disk_path.display()
            )))?;

        // Make the copy writable — Nix store originals are read-only (0444),
        // and tokio::fs::copy preserves permissions. CH needs rw access.
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o644);
            tokio::fs::set_permissions(&writable_disk_path, perms)
                .await
                .map_err(|e| VmError::Internal(format!(
                    "Failed to set writable permissions on {}: {e}",
                    writable_disk_path.display()
                )))?;
        }

        // 4. Serial log path (CH will write console output here)
        let serial_log_path = vm_dir.join("serial.log");

        // 5. Generate a deterministic TAP device name from the VM ID.
        //    Linux limits interface names to 15 chars. "pcr-" prefix (4) +
        //    first 11 chars of the UUID (enough to avoid collisions).
        let tap_name = format!("pcr-{}", &vm_id[..11]);

        // 6. Check if the host bridge actually exists.
        //    Without it (e.g. dev machine, no NixOS host module), we skip
        //    networking entirely — CH won't get --net, TAP won't be attached.
        let network_available = match &self.config.bridge_name {
            Some(bridge) => {
                let exists = Path::new(&format!("/sys/class/net/{bridge}")).exists();
                if !exists {
                    warn!(
                        vm_id = %vm_id,
                        bridge = %bridge,
                        "Bridge device does not exist — VM will boot without network. \
                         Enable the NixOS host module (ch-host.enable = true) for networking."
                    );
                }
                exists
            }
            None => false,
        };

        // 7. Create the TAP device if networking is available.
        //    The worker creates TAPs (not CH) so we control the lifecycle:
        //      - create here in prepare()
        //      - attach to bridge in attach_network() (after CH creates the VM)
        //      - delete in ChProcess::cleanup() when the VM is destroyed
        //    TAP is created persistent so it survives the fd being closed.
        //    CH will re-open it by name via --net tap=<name>.
        if network_available {
            create_tap_device(&tap_name).await.map_err(|e| {
                VmError::Internal(format!(
                    "Failed to create TAP device {tap_name}: {e}"
                ))
            })?;
            info!(
                vm_id = %vm_id,
                tap = %tap_name,
                "TAP device created for VM"
            );
        }

        // 8. Store prepared state for build_config() and spawn()
        let prepared = PreparedVm {
            writable_disk_path,
            serial_log_path,
            vm_dir,
            tap_name,
            network_available,
        };
        self.prepared
            .lock()
            .expect("prepared lock poisoned")
            .insert(vm_id.to_string(), prepared);

        Ok(())
    }

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

        // 4. Look up the VM dir from prepared state
        let vm_dir = self
            .prepared
            .lock()
            .expect("prepared lock poisoned")
            .get(vm_id)
            .map(|p| p.vm_dir.clone())
            .unwrap_or_else(|| self.config.socket_dir.join(vm_id));

        // 5. Spawn the CH process, redirecting stderr+stdout to a log file
        //    so we can diagnose crashes (CH exits silently otherwise).
        let ch_log_path = vm_dir.join("cloud-hypervisor.log");
        let ch_log_file = std::fs::File::create(&ch_log_path)
            .map_err(|e| VmError::ProcessFailed(format!(
                "Failed to create CH log file {}: {e}",
                ch_log_path.display()
            )))?;
        let stderr_file = ch_log_file
            .try_clone()
            .map_err(|e| VmError::ProcessFailed(format!(
                "Failed to clone CH log file handle: {e}"
            )))?;

        info!(
            vm_id = %vm_id,
            ch_binary = %self.config.ch_binary.display(),
            socket = %socket_path.display(),
            log_path = %ch_log_path.display(),
            "Spawning cloud-hypervisor"
        );

        let child = Command::new(&self.config.ch_binary)
            .arg("--api-socket")
            .arg(&socket_path)
            .stdout(std::process::Stdio::from(ch_log_file))
            .stderr(std::process::Stdio::from(stderr_file))
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                VmError::ProcessFailed(format!(
                    "Failed to spawn {}: {e}",
                    self.config.ch_binary.display()
                ))
            })?;

        // 6. Wait for socket to appear
        Self::wait_for_socket(&socket_path, self.config.socket_timeout).await?;

        // 7. Look up the TAP name from prepared state (if networking is enabled)
        let tap_name = self
            .prepared
            .lock()
            .expect("prepared lock poisoned")
            .get(vm_id)
            .filter(|p| p.network_available)
            .map(|p| p.tap_name.clone());

        // 8. Create the REST client and process handle
        let client = CloudHypervisor::new(&socket_path);
        let process = ChProcess {
            child,
            socket_path: socket_path.clone(),
            vm_dir,
            tap_name,
        };

        Ok((client, process, socket_path))
    }

    fn build_config(&self, vm_id: &str, spec: &VmSpec) -> ChVmConfig {
        let boot_vcpus = spec.cpu() as u8;

        // Look up per-VM prepared state for writable disk and serial log paths.
        let prepared = self
            .prepared
            .lock()
            .expect("prepared lock poisoned");
        let prepared_vm = prepared.get(vm_id);

        // Use the writable disk copy if available, otherwise fall back to the
        // original store path (for backward compat / tests without prepare).
        let disk_path = prepared_vm
            .map(|p| p.writable_disk_path.to_string_lossy().to_string())
            .unwrap_or_else(|| spec.disk_image_path().to_string());

        // Serial: write to file if we have a prepared path, otherwise Null.
        let serial = prepared_vm
            .map(|p| ChSerialConfig {
                mode: "File".to_string(),
                file: Some(p.serial_log_path.to_string_lossy().to_string()),
            })
            .unwrap_or_else(|| ChSerialConfig {
                mode: "Null".to_string(),
                file: None,
            });

        // Kernel and initrd are read-only — safe to use from the Nix store directly.
        let kernel_path = spec.kernel_path().to_string();
        let initrd_path = spec.initrd_path().to_string();

        let cmdline = spec.cmdline().to_string();

        ChVmConfig {
            cpus: ChCpusConfig {
                boot_vcpus,
                max_vcpus: boot_vcpus,
            },
            memory: ChMemoryConfig {
                size: u64::from(spec.memory_mb()) * 1024 * 1024,
            },
            payload: Some(ChPayloadConfig {
                kernel: kernel_path,
                cmdline: Some(cmdline),
                initramfs: Some(initrd_path),
            }),
            disks: vec![ChDiskConfig {
                path: disk_path,
                readonly: Some(false),
                direct: None,
            }],
            net: if prepared_vm.is_some_and(|p| p.network_available) {
                // Tell CH to create a TAP device with a known name so we
                // can attach it to the host bridge between create and boot.
                let tap = prepared_vm
                    .map(|p| p.tap_name.clone())
                    .unwrap_or_else(|| format!("pcr-{}", &vm_id[..vm_id.len().min(11)]));
                Some(vec![ChNetConfig {
                    tap: Some(tap),
                    ip: None,
                    mask: None,
                    mac: None,
                }])
            } else {
                None
            },
            rng: Some(ChRngConfig {
                src: "/dev/urandom".to_string(),
            }),
            console: Some(ChConsoleConfig {
                mode: "Off".to_string(),
            }),
            serial: Some(serial),
        }
    }

    async fn attach_network(&self, vm_id: &str) -> Result<(), VmError> {
        self.attach_tap_to_bridge(vm_id).await
    }
}
