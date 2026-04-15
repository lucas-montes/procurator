use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, de};
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
    id: String,
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
        todo!()
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

struct VmDir {
    vm_dir: PathBuf,
    socket_path: PathBuf,
}

// ── Step 1: Create VM directory ───────────────────────────────────────

/// Creates the per-VM working directory under `<socket_dir>/<vm_id>/`
/// and returns `(vm_dir, socket_path)`.
///
/// The directory layout:
/// ```text
/// <socket_dir>/<vm_id>/
/// ├── ch-api.sock   (created later by CH)
/// ├── root.img      (writable copy, created in step 2)
/// └── serial.log    (created later by CH)
/// ```
async fn create_vm_dir(socket_dir: PathBuf, vm_id: &str) -> Result<VmDir, VmError> {
    let vm_dir = socket_dir.join(vm_id);
    debug!(vm_id = %vm_id, vm_dir = %vm_dir.display(), "creating VM directory");
    tokio::fs::create_dir_all(&vm_dir).await.map_err(|e| {
        VmError::Internal(format!("failed to create VM dir {}: {e}", vm_dir.display()))
    })?;

    let socket_path = vm_dir.join("ch-api.sock");
    debug!(vm_dir = %vm_dir.display(), socket = %socket_path.display(), "VM directory created");
    Ok(VmDir {
        vm_dir,
        socket_path,
    })
}

/// Generates a TAP device name from a VM id.
///
/// Uses `tap-` prefix + first 8 hex chars of the UUID (no hyphens)
/// to stay within the `IFNAMSIZ` limit of 15 characters.
///
/// Example: `019712ab-1234-...` → `tap-019712ab`
fn tap_name_from_id(vm_id: &str) -> String {
    let mut hex = String::with_capacity(12);
    hex.push_str("tap-");
    vm_id
        .chars()
        .filter(|c| c != &'-')
        .take(8)
        .for_each(|c| hex.push(c));
    hex
}

trait ArtifactsResolver {
    async fn fetch(source_path: &Path, destination: &Path) -> Result<(), VmError>;
    async fn verify(path: &Path) -> Result<(), VmError>;
}

// TODO: maybe only used for tests, therefor move it to the test part?
struct LocalArtifactResolver;

impl ArtifactsResolver for LocalArtifactResolver {
    async fn fetch(source_path: &Path, destination: &Path) -> Result<(), VmError> {
        if !source_path.exists() {
            return Err(Error::ArtifactsMissing(format!(
                "artifact not found locally: {}",
                source_path.display()
            ))
            .into());
        }

        tokio::fs::copy(source_path, destination)
            .await
            .map_err(|e| {
                VmError::Internal(format!(
                    "failed to copy artifact {} → {}: {e}",
                    source_path.display(),
                    destination.display()
                ))
            })?;

        Ok(())
    }

    async fn verify(path: &Path) -> Result<(), VmError> {
        if !path.exists() {
            return Err(Error::ArtifactsMissing(format!(
                "artifact not found locally: {}",
                path.display()
            ))
            .into());
        }
        Ok(())
    }
}

struct Artifacts {
    writable_disk: PathBuf,
    serial_log: PathBuf,
}

// ── Step 2: Prepare artifacts ─────────────────────────────────────────

/// Verifies that kernel and initramfs exist locally (read-only access)
/// and copies the root disk into the VM directory (writable).
///
/// Returns `(writable_disk_path, serial_log_path)`.
async fn prepare_artifacts<A: ArtifactsResolver>(
    vm_dir: &Path,
    spec: &CreateVmSpecRef<'_>,
) -> Result<Artifacts, VmError> {
    // Verify read-only artifacts exist.
    A::verify(Path::new(spec.kernel())).await?;
    A::verify(Path::new(spec.initramfs())).await?;
    // Copy root disk into VM dir (writable copy).
    let source_disk = Path::new(spec.root_disk());
    A::verify(source_disk).await?;

    let writable_disk = vm_dir.join("root.img");
    A::fetch(source_disk, &writable_disk).await?;

    let serial_log = vm_dir.join("serial.log");

    debug!(
        kernel = %spec.kernel(),
        initramfs = %spec.initramfs(),
        serial_log = %serial_log.display(),
        writable_disk = %writable_disk.display(),
        "artifacts prepared"
    );

    Ok(Artifacts {
        writable_disk,
        serial_log,
    })
}

// ── Step 3: Create TAP + attach to bridge ─────────────────────────────

/// Creates a persistent TAP device and attaches it to the configured bridge.
///
/// Returns the TAP device name. Hard-errors if the bridge does not exist.
async fn setup_networking(bridge_name: &str, vm_id: &str) -> Result<String, VmError> {
    let tap = tap_name_from_id(vm_id);

    create_tap_device(&tap).await?;
    // attach_tap_to_bridge(&tap, bridge_name).await?;

    info!(bridge_name, tap = %tap, "networking configured");
    Ok(tap)
}

// ── Step 4: Finalize VM config ────────────────────────────────────────

/// Takes the capnp-derived config and patches it with runtime paths
/// (writable disk, serial log, TAP name).
fn finalize_config<'a>(
    config: VmConfigRef<'a>,
    writable_disk: &'a str,
    serial_log: &'a str,
    tap_name: &'a str,
) -> VmConfigRef<'a> {
    config.finalize_for_runtime(writable_disk, serial_log, Some(tap_name))
}

// ── Step 5: Spawn CH process ──────────────────────────────────────────

/// Spawns the `cloud-hypervisor` binary, waits for the API socket to
/// appear, then sends the `vm.create` API call.
///
/// Does **not** call `vm.boot` — the caller is responsible for that
/// so boot can happen asynchronously for faster user response.
async fn spawn_and_create(
    vm_id: &str,
    vm_dir: &Path,
    socket_path: &Path,
    tap_name: &str,
    ch_binary: &Path,
    socket_timeout: Duration,
    config: &VmConfigRef<'_>,
) -> Result<(Process, Client), VmError> {
    let log_file = vm_dir.join("cloud-hypervisor.log");

    let child = Command::new(ch_binary)
        .arg("--api-socket")
        .arg(socket_path.as_os_str())
        .arg("--log-file")
        .arg(log_file.as_os_str())
        .arg("-v")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            VmError::ProcessFailed(format!("failed to spawn {}: {e}", ch_binary.display()))
        })?;

    info!(
        vm_id = %vm_id,
        pid = child.id().unwrap_or(0),
        "cloud-hypervisor process spawned"
    );

    // Wait for the API socket to appear.
    wait_for_socket(socket_timeout, socket_path).await?;

    let client = Client::new(socket_path);
    let process = Process::new(
        child,
        socket_path.to_path_buf(),
        vm_dir.to_path_buf(),
        Some(tap_name.to_string()),
    );

    // Send vm.create — configures the VM but does not boot it.
    client
        .create(config)
        .await
        .map_err(|e| VmError::ProcessFailed(format!("vm.create API call failed: {e}")))?;

    info!(vm_id = %vm_id, "vm.create succeeded — VM is ready to boot");
    Ok((process, client))
}

/// Polls for the API socket file to appear on disk, with a timeout.
async fn wait_for_socket(socket_timeout: Duration, socket_path: &Path) -> Result<(), VmError> {
    let start = Instant::now();
    let poll_interval = Duration::from_millis(50);

    loop {
        if socket_path.exists() {
            debug!(socket = %socket_path.display(), "API socket appeared");
            return Ok(());
        }
        if start.elapsed() > socket_timeout {
            return Err(VmError::ProcessFailed(format!(
                "API socket {} did not appear within {:?}",
                socket_path.display(),
                socket_timeout,
            )));
        }
        tokio::time::sleep(poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use capnp::message::{Builder, HeapAllocator};
    use std::path::{Path, PathBuf};

    use crate::ch::dtos::CreateVmSpecRef;
    use crate::vmm::Factory as VmFactory;

    use super::{
        Config, Factory, LocalArtifactResolver, create_vm_dir, finalize_config, prepare_artifacts,
        tap_name_from_id,
    };

    fn test_config(socket_dir: PathBuf) -> Config {
        Config {
            binary_path: PathBuf::from("/bin/true"),
            socket_dir,
            socket_timeout_secs: 2,
            bridge_name: String::from("br0"),
            artifact_sources: Vec::new(),
        }
    }

    fn build_vm_spec_message(
        root_disk: &str,
        kernel: &str,
        initramfs: &str,
    ) -> Builder<HeapAllocator> {
        let mut message = Builder::new_default();
        let mut vm_spec =
            message.init_root::<commands::common_capnp::vm_spec::Builder<commands::ch_capnp::vm_config::Owned>>();
        let mut vm_cfg = vm_spec.reborrow().init_spec();

        {
            let mut cpus = vm_cfg.reborrow().init_cpus();
            cpus.set_boot_vcpus(1);
        }

        {
            let mut memory = vm_cfg.reborrow().init_memory();
            memory.set_size(512 * 1024 * 1024);
        }

        {
            let mut payload = vm_cfg.reborrow().init_payload();
            payload.set_kernel(kernel);
            payload.set_cmdline("console=ttyS0");
            payload.set_initramfs(initramfs);
        }

        {
            let mut disks = vm_cfg.reborrow().init_disks(1);
            let mut disk = disks.reborrow().get(0);
            disk.set_path(root_disk);
        }

        {
            let mut console = vm_cfg.reborrow().init_console();
            console.set_mode("Off");
        }

        {
            let mut serial = vm_cfg.reborrow().init_serial();
            serial.set_mode("Tty");
        }

        message
    }

    fn vm_spec_from_message<'a>(message: &'a Builder<HeapAllocator>) -> CreateVmSpecRef<'a> {
        let reader: commands::common_capnp::vm_spec::Reader<
            'a,
            commands::ch_capnp::vm_config::Owned,
        > = message
            .get_root_as_reader()
            .expect("capnp root reader should be available");
        CreateVmSpecRef::try_from(reader).expect("CreateVmSpecRef conversion should succeed")
    }

    use std::sync::Once;
    use tracing_subscriber::{EnvFilter, fmt};

    static TRACING_INIT: Once = Once::new();

    fn init_tracing() {
        TRACING_INIT.call_once(|| {
            let _ = fmt()
                .with_env_filter(EnvFilter::new("debug"))
                .with_test_writer()
                .try_init();
        });
    }

    #[tokio::test]
    async fn create_vm_dir_creates_directory_and_socket_path() {
        init_tracing();
        let socket_dir = PathBuf::from("tests/data/tmp/sockets");

        let vm_id = "test-vm-001";
        let result = create_vm_dir(socket_dir, vm_id).await.unwrap();

        assert_eq!(
            result.vm_dir.to_str(),
            Some("tests/data/tmp/sockets/test-vm-001")
        );
        assert_eq!(
            result.socket_path,
            PathBuf::from("tests/data/tmp/sockets/test-vm-001/ch-api.sock")
        );
        // Socket file itself is NOT created — CH will create it.
        assert!(!result.socket_path.exists());
        tokio::fs::remove_dir_all(&result.vm_dir)
            .await
            .expect("cleanup should succeed");
    }

    // ── Step 2: Artifact preparation ──────────────────────────────────────

    /// Creates a temp dir with fake kernel, initramfs and root disk files.
    /// Returns `(temp_dir_guard, kernel_path, initramfs_path, root_disk_path)`.
    fn create_fake_artifacts(tmp: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let kernel = tmp.join("kernel");
        let initramfs = tmp.join("initramfs");
        let root_disk = tmp.join("root.img");

        std::fs::write(&kernel, b"fake-kernel").unwrap();
        std::fs::write(&initramfs, b"fake-initramfs").unwrap();
        std::fs::write(&root_disk, b"fake-root-disk").unwrap();

        (kernel, initramfs, root_disk)
    }

    #[tokio::test]
    async fn prepare_artifacts_copies_disk_and_verifies_ro_files() {
        init_tracing();
        let tmp = PathBuf::from("tests/data");
        let (kernel, initramfs, root_disk) = create_fake_artifacts(&tmp);
        let vm_dir = tmp.join("vm-artifacts-test");
        std::fs::create_dir_all(&vm_dir).unwrap();

        let message = build_vm_spec_message(
            root_disk.to_str().unwrap(),
            kernel.to_str().unwrap(),
            initramfs.to_str().unwrap(),
        );
        let spec = vm_spec_from_message(&message);

        let result = prepare_artifacts::<LocalArtifactResolver>(&vm_dir, &spec)
            .await
            .unwrap();

        // Writable disk is a copy inside vm_dir.
        assert_eq!(result.writable_disk, vm_dir.join("root.img"));
        assert!(result.writable_disk.exists());
        assert_eq!(
            std::fs::read(&result.writable_disk).unwrap(),
            b"fake-root-disk"
        );

        // Serial log path is defined but not yet created.
        assert_eq!(result.serial_log, vm_dir.join("serial.log"));
        assert!(!result.serial_log.exists());
    }

    #[tokio::test]
    async fn prepare_artifacts_fails_when_kernel_missing() {
        init_tracing();
        let tmp = PathBuf::from("tests/data");
        let (_kernel, initramfs, root_disk) = create_fake_artifacts(&tmp);
        let vm_dir = tmp.join("vm-kernel-missing");
        std::fs::create_dir_all(&vm_dir).unwrap();

        let message = build_vm_spec_message(
            root_disk.to_str().unwrap(),
            "/nonexistent/kernel",
            initramfs.to_str().unwrap(),
        );
        let spec = vm_spec_from_message(&message);

        let result = prepare_artifacts::<LocalArtifactResolver>(&vm_dir, &spec).await;
        assert!(result.is_err());
    }

    // ── TAP naming ────────────────────────────────────────────────────────

    #[test]
    fn tap_name_strips_hyphens_and_takes_first_8() {
        // v7 UUID example: 019712ab-3c4d-7e5f-8a9b-0c1d2e3f4a5b
        let name = tap_name_from_id("019712ab-3c4d-7e5f-8a9b-0c1d2e3f4a5b");
        assert_eq!(name, "tap-019712ab");
        // IFNAMSIZ = 16 (15 chars + null), our name is "tap-" (4) + 8 = 12
        assert!(name.len() <= 15, "TAP name too long: {name}");
    }


    #[test]
    fn finalize_config_patches_disk_serial_and_net() {
        init_tracing();
        let tmp = PathBuf::from("tests/data");
        let (kernel, initramfs, root_disk) = create_fake_artifacts(&tmp);

        let message = build_vm_spec_message(
            root_disk.to_str().unwrap(),
            kernel.to_str().unwrap(),
            initramfs.to_str().unwrap(),
        );
        let spec = vm_spec_from_message(&message);

        let config = finalize_config(
            spec.vm_config().clone(),
            "/vm/root.img",
            "/vm/serial.log",
            "tap-aabbccdd",
        );

        // Serialize to JSON and verify the fields were patched.
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();

        assert_eq!(json["disks"][0]["path"], "/vm/root.img");
        assert_eq!(json["serial"]["mode"], "File");
        assert_eq!(json["serial"]["file"], "/vm/serial.log");
        assert_eq!(json["console"]["mode"], "Off");
        assert_eq!(json["net"][0]["tap"], "tap-aabbccdd");
    }

    #[tokio::test]
    async fn test_name() {
        init_tracing();

        let message =
            build_vm_spec_message("/path/to/root.img", "/path/to/kernel", "/path/to/initramfs");
        let spec_ref = vm_spec_from_message(&message);
        println!("{:?}", spec_ref);

        let config = test_config(PathBuf::from("/tmp/sockets"));
        let factory = Factory::from(config);

        println!("{:?}", factory);

        factory.create_vm(spec_ref).await.unwrap();
    }
}
