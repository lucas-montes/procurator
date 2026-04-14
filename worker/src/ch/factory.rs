use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::process::Command;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::vmm::{CreateCommand, Error as VmError, Factory as VmFactory, Handle as VmHandle};

use super::{
    client::Client,
    dtos::{CreateVmSpecRef, VmConfigRef},
    process::{Process, create_tap_device, delete_tap_device},
};

pub struct Handle {
    vm_id: String,
    client: Client,
    process: Process,
    socket_path: PathBuf,
}

impl VmHandle for Handle {}

/// The structure responsible to spin up vm managed with cloud hypervisor. We need the socket and the binary to be able to call and communicate
/// with the child processes. We also need to have a bridge name to be able to attach the TAP interfaces to it. I don't really mind cloning it,
/// even if maybe using an Arc would be better? Not, sure, some path we need to modify them to add the uuid to identify each vmm/vm, but not the
/// binary path nor the bridge_name. Also instead of cloning them I should have a function that returns a hadnle, which whil hold the logic
/// to make the vm do stuff?
#[derive(Debug, Clone)]
pub struct Factory {
    socket_dir: PathBuf,
    ch_binary: PathBuf,
    socket_timeout: Duration,
    bridge_name: String,
    artifact_sources: Vec<String>,
}

impl From<Config> for Factory {
    fn from(config: Config) -> Self {
        Self {
            socket_dir: config.socket_dir,
            ch_binary: config.binary_path,
            socket_timeout: Duration::from_secs(config.socket_timeout_secs),
            bridge_name: config.bridge_name,
            artifact_sources: config.artifact_sources,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Error {
    ArtifactsMissing(String),
    InvalidPathUtf8 { field: String, path: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ArtifactsMissing(message) => write!(f, "{message}"),
            Error::InvalidPathUtf8 { field, path } => {
                write!(f, "{field} contains non-UTF8 path: {path}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for VmError {
    fn from(value: Error) -> Self {
        VmError::Internal(value.to_string())
    }
}

struct Artifacts {
    vm_dir: PathBuf,
    serial_log_path: PathBuf,
    writable_disk_path: PathBuf,
}

impl Factory {
    /// We need to get the artifacts generated from the nix build.
    /// TODO: We should have sources to fetch from, probably passed from the config at start.
    /// The way we fetch them is by calling `nix copy --from <source> <paths>` for each source
    /// this implies we have nix installed tho
    async fn fetch_artifacts(&self, spec: &CreateVmSpecRef<'_>) -> Result<(), Error> {
        let mut required_paths = Vec::new();

        if let Some(kernel) = spec.kernel() {
            required_paths.push(kernel);
        }
        if let Some(initramfs) = spec.initramfs() {
            required_paths.push(initramfs);
        }

        let root_disk = spec.root_disk().ok_or_else(|| {
            Error::ArtifactsMissing("VM config must contain at least one disk path".to_string())
        })?;
        required_paths.push(root_disk);

        let mut missing: Vec<&str> = required_paths
            .into_iter()
            .filter(|path| !std::path::Path::new(path).exists())
            .collect();

        if missing.is_empty() {
            return Ok(());
        }

        if self.artifact_sources.is_empty() {
            return Err(Error::ArtifactsMissing(format!(
                "Missing artifacts ({}) and no artifact_sources configured",
                missing.join(", ")
            )));
        }

        //TODO: check if this command is ok or we need something fancier
        // either way it should be in repo_outils/nix
        for source in &self.artifact_sources {
            let mut command = Command::new("nix");
            command.arg("copy").arg("--from").arg(source);
            for path in &missing {
                command.arg(path);
            }
            match command.output().await {
                Ok(output) if output.status.success() => {
                    info!(source = %source, "Fetched artifacts from source");
                    missing.retain(|path| !std::path::Path::new(path).exists());
                    if missing.is_empty() {
                        return Ok(());
                    }
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    warn!(source = %source, stderr = %stderr, "Failed to fetch artifacts from source");
                }
                Err(error) => {
                    warn!(source = %source, error = %error, "Failed to run nix copy for artifacts");
                }
            }
        }

        Err(Error::ArtifactsMissing(format!(
            "Artifacts still missing after fetch attempt: {}",
            missing.join(", ")
        )))
    }

    /// We first create the directory to store the artifacts and later on possibly to create the socket too
    async fn prepare_vm_dir_and_disk(
        &self,
        vm_id: &str,
        spec: &CreateVmSpecRef<'_>,
    ) -> Result<Artifacts, VmError> {
        let vm_dir = self.socket_dir.join(vm_id);
        tokio::fs::create_dir_all(&vm_dir).await.map_err(|e| {
            VmError::ProcessFailed(format!(
                "Failed to create VM directory {}: {e}",
                vm_dir.display()
            ))
        })?;

        let writable_disk_path = vm_dir.join("disk.img");
        let root_disk = spec.root_disk().ok_or_else(|| {
            VmError::Internal("VM config must contain at least one disk path".to_string())
        })?;

        tokio::fs::copy(root_disk, &writable_disk_path)
            .await
            .map_err(|e| {
                VmError::Internal(format!(
                    "Failed to copy disk image from {} to {}: {e}",
                    root_disk,
                    writable_disk_path.display()
                ))
            })?;

        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o644);
            tokio::fs::set_permissions(&writable_disk_path, perms)
                .await
                .map_err(|e| {
                    VmError::Internal(format!(
                        "Failed to set writable permissions on {}: {e}",
                        writable_disk_path.display()
                    ))
                })?;
        }

        let serial_log_path = vm_dir.join("serial.log");
        Ok(Artifacts {
            vm_dir,
            serial_log_path,
            writable_disk_path,
        })
    }

    async fn setup_network(&self, vm_id: &str) -> Result<(String, bool), VmError> {
        let tap_name = format!("pcr-{}", &vm_id[..vm_id.len().min(11)]);
        let bridge_exists =
            std::path::Path::new(&format!("/sys/class/net/{}", self.bridge_name)).exists();
        if !bridge_exists {
            warn!(
                vm_id = %vm_id,
                bridge = %self.bridge_name,
                "Bridge device does not exist — VM will boot without network"
            );
            return Ok((tap_name, false));
        }

        create_tap_device(&tap_name).await.map_err(|e| {
            VmError::Internal(format!("Failed to create TAP device {tap_name}: {e}"))
        })?;
        info!(vm_id = %vm_id, tap = %tap_name, "TAP device created for VM");

        Ok((tap_name, true))
    }

    async fn spawn_ch(
        &self,
        vm_id: &str,
        vm_dir: &std::path::Path,
    ) -> Result<(tokio::process::Child, PathBuf), VmError> {
        tokio::fs::create_dir_all(&self.socket_dir)
            .await
            .map_err(|e| VmError::ProcessFailed(format!("Failed to create socket dir: {e}")))?;

        let socket_path = self.socket_dir.join(format!("{vm_id}.sock"));
        if socket_path.exists() {
            let _ = tokio::fs::remove_file(&socket_path).await;
        }

        let ch_log_path = vm_dir.join("cloud-hypervisor.log");
        let ch_log_file = std::fs::File::create(&ch_log_path).map_err(|e| {
            VmError::ProcessFailed(format!(
                "Failed to create CH log file {}: {e}",
                ch_log_path.display()
            ))
        })?;
        let stderr_file = ch_log_file.try_clone().map_err(|e| {
            VmError::ProcessFailed(format!("Failed to clone CH log file handle: {e}"))
        })?;

        let child = Command::new(&self.ch_binary)
            .arg("--api-socket")
            .arg(&socket_path)
            .stdout(std::process::Stdio::from(ch_log_file))
            .stderr(std::process::Stdio::from(stderr_file))
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                VmError::ProcessFailed(format!("Failed to spawn {}: {e}", self.ch_binary.display()))
            })?;

        Ok((child, socket_path))
    }

    async fn wait_for_socket(path: &std::path::Path, timeout: Duration) -> Result<(), VmError> {
        let start = Instant::now();
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

    // The spec already carries a CH-shaped config from capnp.
    // This step only applies worker-local runtime overrides that cannot be known
    // upstream (writable disk path, host TAP wiring, serial log path, fallback RNG).
    fn finalize_vm_config_for_runtime<'a>(
        &self,
        spec: &'a CreateVmSpecRef<'a>,
        writable_disk_path: &'a str,
        serial_log_path: &'a str,
        tap_name: &'a str,
        network_enabled: bool,
    ) -> VmConfigRef<'a> {
        spec.vm_config().clone().finalize_for_runtime(
            writable_disk_path,
            serial_log_path,
            tap_name,
            network_enabled,
        )
    }

    fn path_to_str<'a>(&self, path: &'a Path, field_name: &str) -> Result<&'a str, Error> {
        path.to_str().ok_or_else(|| Error::InvalidPathUtf8 {
            field: field_name.to_string(),
            path: path.display().to_string(),
        })
    }

    async fn attach_tap_to_bridge(&self, vm_id: &str, tap_name: &str) -> Result<(), VmError> {
        use futures::stream::TryStreamExt;

        async fn link_index(
            handle: &rtnetlink::Handle,
            name: &str,
        ) -> Result<Option<u32>, VmError> {
            let mut links = handle.link().get().match_name(name.to_string()).execute();
            let opt_msg = links
                .try_next()
                .await
                .map_err(|e| VmError::Internal(format!("netlink get failed: {e}")))?;
            Ok(opt_msg.map(|msg| msg.header.index))
        }

        let (connection, handle, _) = rtnetlink::new_connection()
            .map_err(|e| VmError::Internal(format!("netlink connection failed: {e}")))?;
        tokio::spawn(connection);

        let max_attempts = 20;
        for attempt in 1..=max_attempts {
            match link_index(&handle, tap_name).await? {
                Some(tap_index) => {
                    let Some(bridge_index) = link_index(&handle, &self.bridge_name).await? else {
                        return Err(VmError::Internal(format!(
                            "bridge {} not found when attaching TAP",
                            self.bridge_name
                        )));
                    };

                    let attach_result = handle
                        .link()
                        .set(tap_index)
                        .master(bridge_index)
                        .up()
                        .execute()
                        .await;
                    match attach_result {
                        Ok(()) => {
                            info!(
                                vm_id = %vm_id,
                                tap = %tap_name,
                                bridge = %self.bridge_name,
                                attempts = attempt,
                                "TAP attached to bridge"
                            );
                            return Ok(());
                        }
                        Err(e) => {
                            warn!(
                                vm_id = %vm_id,
                                tap = %tap_name,
                                bridge = %self.bridge_name,
                                attempts = attempt,
                                stderr = %e,
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
                        bridge = %self.bridge_name,
                        attempts = attempt,
                        "TAP not visible yet; retrying bridge attach"
                    );
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                None => {
                    warn!(
                        vm_id = %vm_id,
                        tap = %tap_name,
                        bridge = %self.bridge_name,
                        "TAP still missing after retries — VM may have no network"
                    );
                    return Ok(());
                }
            }
        }

        Ok(())
    }

    async fn cleanup_failed_creation(process: &mut Process) {
        let _ = process.kill().await;
        let _ = process.cleanup().await;
    }
}

impl VmFactory for Factory {
    type VmHandle = Handle;
    type Config = Config;
    type BackendConfig = commands::ch_capnp::vm_config::Owned;
    type CreateVmSpec<'a> = CreateVmSpecRef<'a>;

    fn create_id() -> String {
        Uuid::now_v7().to_string()
    }

    async fn create_vm(
        &self,
        source: Self::CreateVmSpec<'_>,
    ) -> Result<CreateCommand<Self>, VmError> {
        let vm_id = Self::create_id();
        debug!(vm_id = %vm_id, "creating VM from spec");
        self.fetch_artifacts(&source).await?;

        let artifacts = self.prepare_vm_dir_and_disk(&vm_id, &source).await?;
        let (tap_name, network_enabled) = self.setup_network(&vm_id).await?;
        let (child, socket_path) = self.spawn_ch(&vm_id, &artifacts.vm_dir).await?;
        Self::wait_for_socket(&socket_path, self.socket_timeout).await?;

        let writable_disk_path =
            self.path_to_str(&artifacts.writable_disk_path, "writable_disk")?;
        let serial_log_path = self.path_to_str(&artifacts.serial_log_path, "serial_log")?;

        let vm_config = self.finalize_vm_config_for_runtime(
            &source,
            &writable_disk_path,
            &serial_log_path,
            &tap_name,
            network_enabled,
        );

        let client = Client::new(&socket_path);
        let mut process = Process::new(
            child,
            socket_path.clone(),
            artifacts.vm_dir,
            network_enabled.then_some(tap_name.clone()),
        );

        if let Err(err) = client.create(&vm_config).await {
            Self::cleanup_failed_creation(&mut process).await;
            return Err(VmError::Hypervisor(format!("Failed to create VM: {err}")));
        }

        if network_enabled {
            self.attach_tap_to_bridge(&vm_id, &tap_name).await?;
        }

        if let Err(err) = client.boot().await {
            Self::cleanup_failed_creation(&mut process).await;
            return Err(VmError::Hypervisor(format!("Failed to boot VM: {err}")));
        }

        let handle = Handle {
            vm_id: vm_id.clone(),
            client,
            process,
            socket_path,
        };

        Ok(CreateCommand::new(handle, vm_id))
    }

    async fn delete_vm(&self, id: &str) -> Result<(), VmError> {
        let socket_path = self.socket_dir.join(format!("{id}.sock"));
        if socket_path.exists() {
            let _ = tokio::fs::remove_file(&socket_path).await;
        }

        let vm_dir = self.socket_dir.join(id);
        if vm_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&vm_dir).await;
        }

        let tap_name = format!("pcr-{}", &id[..id.len().min(11)]);
        let _ = delete_tap_device(&tap_name).await;

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct Config {
    binary_path: PathBuf,
    socket_dir: PathBuf,
    socket_timeout_secs: u64,
    bridge_name: String,
    #[serde(default)]
    artifact_sources: Vec<String>,
}
