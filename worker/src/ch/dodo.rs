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
    process::{Process, attach_tap_to_bridge, create_tap_device},
};

pub struct Handle {
    id: String,
    client: Client,
    process: Process,
    socket_path: PathBuf,
}

impl VmHandle for Handle {}

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

/// Generates a TAP device name from a VM id.
///
/// Uses `tap-` prefix + first 8 hex chars of the UUID (no hyphens)
/// to stay within the `IFNAMSIZ` limit of 15 characters.
///
/// Example: `019712ab-1234-...` → `tap-019712ab`
fn tap_name_from_id(vm_id: &str) -> String {
    let hex: String = vm_id.chars().filter(|c| *c != '-').take(8).collect();
    format!("tap-{hex}")
}

/// Converts a `PathBuf` to a `&str`, returning a descriptive error on non-UTF8.
fn path_to_str<'a>(path: &'a Path, field: &'static str) -> Result<&'a str, Error> {
    path.to_str().ok_or_else(|| Error::InvalidPathUtf8 {
        field: field.to_string(),
        path: path.display().to_string(),
    })
}

impl Factory {
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
    async fn create_vm_dir(&self, vm_id: &str) -> Result<(PathBuf, PathBuf), VmError> {
        let vm_dir = self.socket_dir.join(vm_id);
        tokio::fs::create_dir_all(&vm_dir)
            .await
            .map_err(|e| VmError::Internal(format!("failed to create VM dir {}: {e}", vm_dir.display())))?;

        let socket_path = vm_dir.join("ch-api.sock");
        debug!(vm_dir = %vm_dir.display(), socket = %socket_path.display(), "VM directory created");
        Ok((vm_dir, socket_path))
    }

    // ── Step 2: Prepare artifacts ─────────────────────────────────────────

    /// Verifies that kernel and initramfs exist locally (read-only access)
    /// and copies the root disk into the VM directory (writable).
    ///
    /// Returns `(writable_disk_path, serial_log_path)`.
    async fn prepare_artifacts(
        &self,
        vm_dir: &Path,
        spec: &CreateVmSpecRef<'_>,
    ) -> Result<(PathBuf, PathBuf), VmError> {
        // Verify read-only artifacts exist locally.
        let kernel = Path::new(spec.kernel());
        if !kernel.exists() {
            return Err(Error::ArtifactsMissing(format!(
                "kernel not found locally: {}",
                kernel.display()
            ))
            .into());
        }

        let initramfs = Path::new(spec.initramfs());
        if !initramfs.exists() {
            return Err(Error::ArtifactsMissing(format!(
                "initramfs not found locally: {}",
                initramfs.display()
            ))
            .into());
        }

        // Copy root disk into VM dir (writable copy).
        let source_disk = Path::new(spec.root_disk());
        if !source_disk.exists() {
            return Err(Error::ArtifactsMissing(format!(
                "root disk not found: {}",
                source_disk.display()
            ))
            .into());
        }

        let writable_disk = vm_dir.join("root.img");
        tokio::fs::copy(source_disk, &writable_disk)
            .await
            .map_err(|e| {
                VmError::Internal(format!(
                    "failed to copy root disk {} → {}: {e}",
                    source_disk.display(),
                    writable_disk.display()
                ))
            })?;

        let serial_log = vm_dir.join("serial.log");

        info!(
            kernel = %kernel.display(),
            initramfs = %initramfs.display(),
            writable_disk = %writable_disk.display(),
            "artifacts prepared"
        );

        Ok((writable_disk, serial_log))
    }

    // ── Step 3: Create TAP + attach to bridge ─────────────────────────────

    /// Creates a persistent TAP device and attaches it to the configured bridge.
    ///
    /// Returns the TAP device name. Hard-errors if the bridge does not exist.
    async fn setup_networking(&self, vm_id: &str) -> Result<String, VmError> {
        let tap = tap_name_from_id(vm_id);

        create_tap_device(&tap).await?;
        attach_tap_to_bridge(&tap, &self.bridge_name).await?;

        info!(tap = %tap, bridge = %self.bridge_name, "networking configured");
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
        config.finalize_for_runtime(writable_disk, serial_log, tap_name, true)
    }

    // ── Step 5: Spawn CH process ──────────────────────────────────────────

    /// Spawns the `cloud-hypervisor` binary, waits for the API socket to
    /// appear, then sends the `vm.create` API call.
    ///
    /// Does **not** call `vm.boot` — the caller is responsible for that
    /// so boot can happen asynchronously for faster user response.
    async fn spawn_and_create(
        &self,
        vm_id: &str,
        vm_dir: &Path,
        socket_path: &Path,
        tap_name: &str,
        config: &VmConfigRef<'_>,
    ) -> Result<(Process, Client), VmError> {
        let log_file = vm_dir.join("cloud-hypervisor.log");

        let child = Command::new(&self.ch_binary)
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
                VmError::ProcessFailed(format!(
                    "failed to spawn {}: {e}",
                    self.ch_binary.display()
                ))
            })?;

        info!(
            vm_id = %vm_id,
            pid = child.id().unwrap_or(0),
            "cloud-hypervisor process spawned"
        );

        // Wait for the API socket to appear.
        self.wait_for_socket(socket_path).await?;

        let client = Client::new(socket_path);
        let process = Process::new(
            child,
            socket_path.to_path_buf(),
            vm_dir.to_path_buf(),
            Some(tap_name.to_string()),
        );

        // Send vm.create — configures the VM but does not boot it.
        client.create(config).await.map_err(|e| {
            VmError::ProcessFailed(format!("vm.create API call failed: {e}"))
        })?;

        info!(vm_id = %vm_id, "vm.create succeeded — VM is ready to boot");
        Ok((process, client))
    }

    /// Polls for the API socket file to appear on disk, with a timeout.
    async fn wait_for_socket(&self, socket_path: &Path) -> Result<(), VmError> {
        let start = Instant::now();
        let poll_interval = Duration::from_millis(50);

        loop {
            if socket_path.exists() {
                debug!(socket = %socket_path.display(), "API socket appeared");
                return Ok(());
            }
            if start.elapsed() > self.socket_timeout {
                return Err(VmError::ProcessFailed(format!(
                    "API socket {} did not appear within {:?}",
                    socket_path.display(),
                    self.socket_timeout,
                )));
            }
            tokio::time::sleep(poll_interval).await;
        }
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

        // Step 1: Create VM directory and socket path.
        let (vm_dir, socket_path) = self.create_vm_dir(&vm_id).await?;

        // Step 2: Prepare artifacts (copy disk, verify kernel/initramfs).
        let (writable_disk, serial_log) = self.prepare_artifacts(&vm_dir, &source).await?;

        // Step 3: Create TAP device and attach to bridge.
        let tap_name = self.setup_networking(&vm_id).await?;

        // Step 4: Finalize VM config with runtime paths.
        let writable_disk_str = path_to_str(&writable_disk, "writable_disk")?;
        let serial_log_str = path_to_str(&serial_log, "serial_log")?;
        let config = Self::finalize_config(
            source.vm_config().clone(),
            writable_disk_str,
            serial_log_str,
            &tap_name,
        );

        // Step 5: Spawn CH process, wait for socket, send vm.create.
        let (process, client) = self
            .spawn_and_create(&vm_id, &vm_dir, &socket_path, &tap_name, &config)
            .await?;

        let handle = Handle {
            id: vm_id,
            client,
            process,
            socket_path,
        };

        Ok(CreateCommand::new(handle))
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

#[cfg(test)]
mod tests {
    use capnp::message::{Builder, HeapAllocator};
    use std::path::PathBuf;
    use tempfile::TempDir;

    use crate::ch::dtos::CreateVmSpecRef;
    use crate::vmm::Factory as VmFactory;

    use super::{Config, Factory, tap_name_from_id};

    // ── Helpers ───────────────────────────────────────────────────────────

    fn test_config(socket_dir: PathBuf) -> Config {
        Config {
            binary_path: PathBuf::from("/bin/true"),
            socket_dir,
            socket_timeout_secs: 2,
            bridge_name: String::from("br0"),
            artifact_sources: Vec::new(),
        }
    }

    /// Creates a temp dir with fake kernel, initramfs and root disk files.
    /// Returns `(temp_dir_guard, kernel_path, initramfs_path, root_disk_path)`.
    fn create_fake_artifacts(tmp: &TempDir) -> (PathBuf, PathBuf, PathBuf) {
        let kernel = tmp.path().join("kernel");
        let initramfs = tmp.path().join("initramfs");
        let root_disk = tmp.path().join("root.img");

        std::fs::write(&kernel, b"fake-kernel").unwrap();
        std::fs::write(&initramfs, b"fake-initramfs").unwrap();
        std::fs::write(&root_disk, b"fake-root-disk").unwrap();

        (kernel, initramfs, root_disk)
    }

    fn build_vm_spec_message(
        root_disk: &str,
        kernel: &str,
        initramfs: &str,
    ) -> Builder<HeapAllocator> {
        let mut message = Builder::new_default();
        let mut vm_spec = message.init_root::<
            commands::common_capnp::vm_spec::Builder<commands::ch_capnp::vm_config::Owned>,
        >();
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

    // ── TAP naming ────────────────────────────────────────────────────────

    #[test]
    fn tap_name_strips_hyphens_and_takes_first_8() {
        // v7 UUID example: 019712ab-3c4d-7e5f-8a9b-0c1d2e3f4a5b
        let name = tap_name_from_id("019712ab-3c4d-7e5f-8a9b-0c1d2e3f4a5b");
        assert_eq!(name, "tap-019712ab");
    }

    #[test]
    fn tap_name_within_ifnamsiz() {
        let name = tap_name_from_id("019712ab-3c4d-7e5f-8a9b-0c1d2e3f4a5b");
        // IFNAMSIZ = 16 (15 chars + null), our name is "tap-" (4) + 8 = 12
        assert!(name.len() <= 15, "TAP name too long: {name}");
    }

    // ── Step 1: VM directory creation ─────────────────────────────────────

    #[tokio::test]
    async fn create_vm_dir_creates_directory_and_socket_path() {
        init_tracing();
        let tmp = TempDir::new().unwrap();
        let config = test_config(tmp.path().to_path_buf());
        let factory = Factory::from(config);

        let vm_id = "test-vm-001";
        let (vm_dir, socket_path) = factory.create_vm_dir(vm_id).await.unwrap();

        assert!(vm_dir.exists(), "VM dir should exist");
        assert_eq!(vm_dir, tmp.path().join(vm_id));
        assert_eq!(socket_path, vm_dir.join("ch-api.sock"));
        // Socket file itself is NOT created — CH will create it.
        assert!(!socket_path.exists());
    }

    #[tokio::test]
    async fn create_vm_dir_is_idempotent() {
        init_tracing();
        let tmp = TempDir::new().unwrap();
        let config = test_config(tmp.path().to_path_buf());
        let factory = Factory::from(config);

        let vm_id = "test-vm-idem";
        let (dir1, _) = factory.create_vm_dir(vm_id).await.unwrap();
        let (dir2, _) = factory.create_vm_dir(vm_id).await.unwrap();
        assert_eq!(dir1, dir2);
        assert!(dir1.exists());
    }

    // ── Step 2: Artifact preparation ──────────────────────────────────────

    #[tokio::test]
    async fn prepare_artifacts_copies_disk_and_verifies_ro_files() {
        init_tracing();
        let tmp = TempDir::new().unwrap();
        let (kernel, initramfs, root_disk) = create_fake_artifacts(&tmp);
        let vm_dir = tmp.path().join("vm-artifacts-test");
        std::fs::create_dir_all(&vm_dir).unwrap();

        let config = test_config(tmp.path().to_path_buf());
        let factory = Factory::from(config);

        let message = build_vm_spec_message(
            root_disk.to_str().unwrap(),
            kernel.to_str().unwrap(),
            initramfs.to_str().unwrap(),
        );
        let spec = vm_spec_from_message(&message);

        let (writable_disk, serial_log) = factory.prepare_artifacts(&vm_dir, &spec).await.unwrap();

        // Writable disk is a copy inside vm_dir.
        assert_eq!(writable_disk, vm_dir.join("root.img"));
        assert!(writable_disk.exists());
        assert_eq!(
            std::fs::read(&writable_disk).unwrap(),
            b"fake-root-disk"
        );

        // Serial log path is defined but not yet created.
        assert_eq!(serial_log, vm_dir.join("serial.log"));
        assert!(!serial_log.exists());
    }

    #[tokio::test]
    async fn prepare_artifacts_fails_when_kernel_missing() {
        init_tracing();
        let tmp = TempDir::new().unwrap();
        let (_kernel, initramfs, root_disk) = create_fake_artifacts(&tmp);
        let vm_dir = tmp.path().join("vm-kernel-missing");
        std::fs::create_dir_all(&vm_dir).unwrap();

        let config = test_config(tmp.path().to_path_buf());
        let factory = Factory::from(config);

        let message = build_vm_spec_message(
            root_disk.to_str().unwrap(),
            "/nonexistent/kernel",
            initramfs.to_str().unwrap(),
        );
        let spec = vm_spec_from_message(&message);

        let result = factory.prepare_artifacts(&vm_dir, &spec).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("kernel not found"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn prepare_artifacts_fails_when_initramfs_missing() {
        init_tracing();
        let tmp = TempDir::new().unwrap();
        let (kernel, _initramfs, root_disk) = create_fake_artifacts(&tmp);
        let vm_dir = tmp.path().join("vm-initramfs-missing");
        std::fs::create_dir_all(&vm_dir).unwrap();

        let config = test_config(tmp.path().to_path_buf());
        let factory = Factory::from(config);

        let message = build_vm_spec_message(
            root_disk.to_str().unwrap(),
            kernel.to_str().unwrap(),
            "/nonexistent/initramfs",
        );
        let spec = vm_spec_from_message(&message);

        let result = factory.prepare_artifacts(&vm_dir, &spec).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("initramfs not found"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn prepare_artifacts_fails_when_root_disk_missing() {
        init_tracing();
        let tmp = TempDir::new().unwrap();
        let (kernel, initramfs, _root_disk) = create_fake_artifacts(&tmp);
        let vm_dir = tmp.path().join("vm-disk-missing");
        std::fs::create_dir_all(&vm_dir).unwrap();

        let config = test_config(tmp.path().to_path_buf());
        let factory = Factory::from(config);

        let message = build_vm_spec_message(
            "/nonexistent/root.img",
            kernel.to_str().unwrap(),
            initramfs.to_str().unwrap(),
        );
        let spec = vm_spec_from_message(&message);

        let result = factory.prepare_artifacts(&vm_dir, &spec).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("root disk not found"),
            "unexpected error: {err}"
        );
    }

    // ── Step 4: Config finalization ───────────────────────────────────────

    #[test]
    fn finalize_config_patches_disk_serial_and_net() {
        init_tracing();
        let tmp = TempDir::new().unwrap();
        let (kernel, initramfs, root_disk) = create_fake_artifacts(&tmp);

        let message = build_vm_spec_message(
            root_disk.to_str().unwrap(),
            kernel.to_str().unwrap(),
            initramfs.to_str().unwrap(),
        );
        let spec = vm_spec_from_message(&message);

        let config = Factory::finalize_config(
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

    // ── Step 5: Socket wait timeout ───────────────────────────────────────

    #[tokio::test]
    async fn wait_for_socket_times_out_when_no_socket() {
        init_tracing();
        let tmp = TempDir::new().unwrap();
        let mut config = test_config(tmp.path().to_path_buf());
        config.socket_timeout_secs = 0; // immediate timeout
        let factory = Factory::from(config);

        let missing_socket = tmp.path().join("nonexistent.sock");
        let result = factory.wait_for_socket(&missing_socket).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("did not appear"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn wait_for_socket_succeeds_when_file_exists() {
        init_tracing();
        let tmp = TempDir::new().unwrap();
        let config = test_config(tmp.path().to_path_buf());
        let factory = Factory::from(config);

        let socket = tmp.path().join("test.sock");
        std::fs::write(&socket, b"").unwrap(); // pre-create the file

        let result = factory.wait_for_socket(&socket).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wait_for_socket_succeeds_when_file_appears_later() {
        init_tracing();
        let tmp = TempDir::new().unwrap();
        let config = test_config(tmp.path().to_path_buf());
        let factory = Factory::from(config);

        let socket = tmp.path().join("delayed.sock");
        let socket_clone = socket.clone();

        // Spawn a task that creates the socket after a short delay.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            std::fs::write(&socket_clone, b"").unwrap();
        });

        let result = factory.wait_for_socket(&socket).await;
        assert!(result.is_ok());
    }

    // ── Step 5: Spawn with /bin/true (no real CH) ─────────────────────────
    // /bin/true exits immediately and never creates a socket, so this tests
    // the timeout path of spawn_and_create.

    #[tokio::test]
    async fn spawn_and_create_fails_when_binary_exits_before_socket() {
        init_tracing();
        let tmp = TempDir::new().unwrap();
        let vm_dir = tmp.path().join("vm-spawn-fail");
        std::fs::create_dir_all(&vm_dir).unwrap();
        let socket = vm_dir.join("ch-api.sock");

        let mut config = test_config(tmp.path().to_path_buf());
        config.binary_path = PathBuf::from("/bin/true"); // exits 0 immediately
        config.socket_timeout_secs = 1;
        let factory = Factory::from(config);

        let (kernel, initramfs, root_disk) = create_fake_artifacts(&tmp);
        let message = build_vm_spec_message(
            root_disk.to_str().unwrap(),
            kernel.to_str().unwrap(),
            initramfs.to_str().unwrap(),
        );
        let spec = vm_spec_from_message(&message);
        let finalized = Factory::finalize_config(
            spec.vm_config().clone(),
            vm_dir.join("root.img").to_str().unwrap(),
            vm_dir.join("serial.log").to_str().unwrap(),
            "tap-deadbeef",
        );

        let result = factory
            .spawn_and_create("test-vm", &vm_dir, &socket, "tap-deadbeef", &finalized)
            .await;
        assert!(result.is_err(), "should fail because socket never appears");
    }

    #[tokio::test]
    async fn spawn_and_create_fails_when_binary_not_found() {
        init_tracing();
        let tmp = TempDir::new().unwrap();
        let vm_dir = tmp.path().join("vm-no-binary");
        std::fs::create_dir_all(&vm_dir).unwrap();
        let socket = vm_dir.join("ch-api.sock");

        let mut config = test_config(tmp.path().to_path_buf());
        config.binary_path = PathBuf::from("/nonexistent/cloud-hypervisor");
        let factory = Factory::from(config);

        let (kernel, initramfs, root_disk) = create_fake_artifacts(&tmp);
        let message = build_vm_spec_message(
            root_disk.to_str().unwrap(),
            kernel.to_str().unwrap(),
            initramfs.to_str().unwrap(),
        );
        let spec = vm_spec_from_message(&message);
        let finalized = Factory::finalize_config(
            spec.vm_config().clone(),
            vm_dir.join("root.img").to_str().unwrap(),
            vm_dir.join("serial.log").to_str().unwrap(),
            "tap-deadbeef",
        );

        let result = factory
            .spawn_and_create("test-vm", &vm_dir, &socket, "tap-deadbeef", &finalized)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("failed to spawn"),
            "unexpected error: {err}"
        );
    }
}
