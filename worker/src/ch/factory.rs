use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use tokio::process::{Child, Command};
use tracing::{debug, error, info};
use uuid::Uuid;

use crate::ch::dtos::NetConfigRef;
use crate::ch::tap::Tap;
use crate::vmm::{CreateCommand, Error as VmError, Factory as VmFactory};

use super::{client::Client, dtos::CreateVmSpecRef, handle::Handle, ip_allocator::IpAllocator};

/// The structure responsible to spin up vm managed with cloud hypervisor. We need the socket and the binary to be able to call and communicate
/// with the child processes. We also need to have a bridge name to be able to attach the TAP interfaces to it. I don't really mind cloning it,
/// even if maybe using an Arc would be better? Not, sure, some path we need to modify them to add the uuid to identify each vmm/vm, but not the
/// binary path nor the bridge_name. Also instead of cloning them I should have a function that returns a hadnle, which whil hold the logic
/// to make the vm do stuff?
#[derive(Debug, Clone)]
pub struct Factory {
    runtime_dir: PathBuf,
    state_dir: PathBuf,
    ch_binary: PathBuf,
    bridge_name: String,
    /// Gateway IP pushed into the guest via the `procurator.gw=` cmdline
    /// token. Must be the address the host bridge (`bridge_name`) carries.
    bridge_gateway: Ipv4Addr,
    ip_allocator: IpAllocator,
    artifact_sources: Vec<String>,
}

impl Factory {
    pub fn new(config: Config, db: crate::database::Database) -> Self {
        //TODO: I don't like this, but as we don't have an interface and I don't really know what the interafce should look like, let it be
        // I'll make it more generic once the ArtifactsResolver thing is also generic and implemented here
        let ip_allocator = IpAllocator::new(
            db,
            config.ip_pool_start,
            config.ip_pool_end,
            config.ip_netmask,
        );

        Self {
            runtime_dir: config.runtime_dir,
            state_dir: config.state_dir,
            ch_binary: config.binary_path,
            bridge_name: config.bridge_name,
            bridge_gateway: config.bridge_gateway,
            ip_allocator,
            artifact_sources: config.artifact_sources,
        }
    }
}

#[derive(Debug)]
enum Error {
    ArtifactsMissing(String),
    InvalidPathUtf8 {
        field: String,
        path: String,
    },
    /// Errors originating from TAP management code.
    Tap(crate::ch::tap::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ArtifactsMissing(message) => write!(f, "{message}"),
            Error::InvalidPathUtf8 { field, path } => {
                write!(f, "{field} contains non-UTF8 path: {path}")
            }
            Error::Tap(e) => write!(f, "tap error: {}", e),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for VmError {
    fn from(value: Error) -> Self {
        VmError::Internal(value.to_string())
    }
}

impl From<crate::ch::tap::Error> for Error {
    fn from(e: crate::ch::tap::Error) -> Self {
        Error::Tap(e)
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
        spec: Self::CreateVmSpec<'_>,
    ) -> Result<CreateCommand<Self>, VmError> {
        let vm_id = Self::create_id();

        info!(
            vm_id = %vm_id,
            kernel = %spec.kernel(),
            initramfs = %spec.initramfs(),
            root_disk = %spec.root_disk(),
            runtime_dir = %self.runtime_dir.display(),
            state_dir = %self.state_dir.display(),
            ch_binary = %self.ch_binary.display(),
            bridge_name = %self.bridge_name,
            artifact_sources = self.artifact_sources.len(),
            "starting VM creation"
        );

        let vm_dir = create_vm_dir(&self.runtime_dir, &vm_id).await?;
        debug!(vm_id = %vm_id, vm_dir = %vm_dir.vm_dir.display(), "VM directory created");

        let artifacts = prepare_artifacts::<LocalArtifactResolver>(&vm_dir.vm_dir, &spec).await?;

        debug!(
            vm_id = %vm_id,
            kernel = %spec.kernel(),
            initramfs = %spec.initramfs(),
            serial_log = %artifacts.serial_log.display(),
            writable_disk = %artifacts.writable_disk.display(),
            "artifacts prepared"
        );

        // Let the kernel assign automatically the TAP name for now and let's see what happens
        // let tap_name = tap_name_from_id(&vm_id);
        // let tap = Tap::new(Some(&tap_name))
        let tap = Tap::new(None)
            .map_err(Error::from)?
            .persist()
            .map_err(Error::from)?
            .attach_to_bridge(self.bridge_name.clone())
            .await
            .map_err(Error::from)?;

        let tap_name = tap.name();

        debug!(%vm_id, %tap_name, "tap create and attached to bridge");

        // we pass vm_id only to debug if something goes wrong
        let child = spawn_cloud_hypervisor(&vm_id, &vm_dir, &self.ch_binary).await?;

        let lease = self
            .ip_allocator
            .reserve(&vm_id)
            .await
            .map_err(|err| VmError::Internal(err.to_string()))?;

        debug!(
            vm_id = %vm_id,
            ip = %lease.ip(),
            mask = %lease.mask(),
            mac = %lease.mac(),
            "static IP lease reserved"
        );

        let vm_leased_ip = lease.ip().to_string();

        // Generate a random password for OpenCode server
        let opencode_password = uuid::Uuid::new_v4().to_string();

        // Store the password in the database
        if let Err(e) = self
            .ip_allocator
            .store_opencode_password(&vm_id, &opencode_password)
            .await
        {
            tracing::error!(vm_id = %vm_id, error = %e, "Failed to store opencode password");
        }

        // The capnp payload carries the image-specific base cmdline produced
        // by the flake's `artifacts` derivation (kernel params + `init=`
        // with the NixOS toplevel hash). `finalize_for_runtime` layers all
        // per-VM mutations on top in a single pass.
        let vm_config = spec.vm_config().finalize_for_runtime(
            lease.ip(),
            &self.bridge_gateway,
            lease.mask(),
            artifacts.writable_disk(),
            artifacts.serial_log(),
            NetConfigRef::new(&tap_name, lease.mac()),
            &opencode_password,
        );

        let client = Client::new(vm_dir.socket_path);

        client.create(&vm_config).await.map_err(|e| {
            error!(vm_id = %vm_id, ?e, "vm.create API call failed");
            VmError::ProcessFailed(format!("vm.create API call failed: {e}"))
        })?;

        info!(vm_id = %vm_id, "vm.create succeeded — VM is ready to boot");

        let handle = Handle::new(
            client,
            child,
            vm_dir.vm_dir,
            artifacts.writable_disk,
            tap,
            vm_id.clone(),
            self.ip_allocator.clone(),
            vm_leased_ip,
        );
        Ok(CreateCommand::new(handle, vm_id))
    }
}

#[derive(Debug, Deserialize)]
pub struct Config {
    binary_path: PathBuf,
    runtime_dir: PathBuf,
    state_dir: PathBuf,
    bridge_name: String,
    /// Address of the host bridge. Pushed into every guest as the default
    /// gateway via the `procurator.gw=` cmdline token (parsed in stage 2 by
    /// the `procurator-netcfg` systemd unit baked into the image). Must match
    /// the IP actually assigned to `bridge_name` on the host (see
    /// `nix/modules/worker/vmm.nix`).
    bridge_gateway: Ipv4Addr,
    ip_pool_start: Ipv4Addr,
    ip_pool_end: Ipv4Addr,
    ip_netmask: Ipv4Addr,
    #[serde(default)]
    artifact_sources: Vec<String>, //TODO: this hsould probably come from the ArtifactsResolver or something
}

impl Config {
    pub fn state_db_path(&self) -> std::path::PathBuf {
        self.state_dir.join("worker.sqlite")
    }
}

struct VmDir {
    vm_dir: PathBuf,
    socket_path: PathBuf,
}

/// Creates the per-VM working directory under `<runtime_dir>/<vm_id>/`
/// and returns `(vm_dir, socket_path)`.
///
/// The directory layout:
/// ```text
/// <runtime_dir>/<vm_id>/
/// ├── ch-api.sock   (created later by CH)
/// ├── root.img      (writable copy, created in step 2)
/// └── serial.log    (created later by CH)
/// ```
async fn create_vm_dir(runtime_dir: &Path, vm_id: &str) -> Result<VmDir, VmError> {
    let vm_dir = runtime_dir.join(vm_id);
    debug!(vm_id = %vm_id, vm_dir = %vm_dir.display(), "creating VM directory");
    if let Err(err) = tokio::fs::create_dir_all(&vm_dir).await {
        error!(
            vm_id = %vm_id,
            runtime_dir = %runtime_dir.display(),
            ?err,
            "Failed creating runtime VM directory"
        );
        return Err(VmError::Internal(format!(
            "failed to create VM dir {}",
            vm_dir.display()
        )));
    }

    let socket_path = vm_dir.join("ch-api.sock");
    debug!(vm_dir = %vm_dir.display(), socket = %socket_path.display(), "VM directory created");
    Ok(VmDir {
        vm_dir,
        socket_path,
    })
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
            error!(
                source = %source_path.display(),
                destination = %destination.display(),
                "artifact source missing during fetch"
            );
            return Err(Error::ArtifactsMissing(format!(
                "artifact not found locally: {}",
                source_path.display()
            ))
            .into());
        }

        debug!(
            source = %source_path.display(),
            destination = %destination.display(),
            "copying artifact"
        );

        tokio::fs::copy(source_path, destination)
            .await
            .map_err(|e| {
                error!(
                    source = %source_path.display(),
                    destination = %destination.display(),
                    ?e,
                    "artifact copy failed"
                );
                VmError::Internal(format!(
                    "failed to copy artifact {} → {}: {e}",
                    source_path.display(),
                    destination.display()
                ))
            })?;

        debug!(
            source = %source_path.display(),
            destination = %destination.display(),
            "artifact copy completed"
        );

        Ok(())
    }

    async fn verify(path: &Path) -> Result<(), VmError> {
        if !path.exists() {
            error!(path = %path.display(), "required artifact path missing");
            return Err(Error::ArtifactsMissing(format!(
                "artifact not found locally: {}",
                path.display()
            ))
            .into());
        }
        debug!(path = %path.display(), "artifact path exists");
        Ok(())
    }
}

// TODO: rename this to somehting more precise, artifacts is very vague and used for also the kernel + initrd+ disk for the vm
struct Artifacts {
    writable_disk: PathBuf,
    serial_log: PathBuf,
}

impl Artifacts {
    fn writable_disk(&self) -> &str {
        self.writable_disk
            .to_str()
            .ok_or_else(|| Error::InvalidPathUtf8 {
                field: "writable disk".to_string(),
                path: self.writable_disk.display().to_string(),
            })
            .expect("let's deal with that shit later")
    }

    fn serial_log(&self) -> &str {
        self.serial_log
            .to_str()
            .ok_or_else(|| Error::InvalidPathUtf8 {
                field: "serial_log".to_string(),
                path: self.serial_log.display().to_string(),
            })
            .expect("let's deal with that shit later")
    }
}

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

/// Spawns the `cloud-hypervisor` process with the given socket path and log file.
async fn spawn_cloud_hypervisor(
    vm_id: &str,
    vm_dir_setings: &VmDir,
    ch_binary: &Path,
) -> Result<Child, VmError> {
    //TODO: not sure if this log file is necessary at all or how to use it properly
    let log_file = vm_dir_setings.vm_dir.join("cloud-hypervisor.log");

    debug!(
        vm_dir = %vm_dir_setings.vm_dir.display(),
        socket_path = %vm_dir_setings.socket_path.display(),
        log_file = %log_file.display(),
        ch_binary = %ch_binary.display(),
        "spawning cloud-hypervisor"
    );

    let mut child = Command::new(ch_binary)
        .arg("--api-socket")
        .arg(vm_dir_setings.socket_path.as_os_str())
        .arg("--log-file")
        .arg(log_file.as_os_str())
        .arg("-v")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            error!(ch_binary = %ch_binary.display(), ?e, "failed to spawn cloud-hypervisor");
            VmError::ProcessFailed(format!("failed to spawn {}: {e}", ch_binary.display()))
        })?;

    let pid = child.id().ok_or_else(||{
        error!( "spawned cloud-hypervisor process has no PID, it means that it finished, which shouldn't be the case");
        VmError::ProcessFailed(format!("cloud-hypervisor process finished immediately after spawning"))
        })?;

    // Wait for CH to create its API socket before returning. Without this, the
    // first vm.create call races the CH startup and fails with ENOENT.
    //
    // Bounded retry: poll every 20ms for up to 2s. If CH hasn't created the
    // socket by then, something is wrong — kill the child and fail.
    let socket_path = &vm_dir_setings.socket_path;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !socket_path.exists() {
        if std::time::Instant::now() >= deadline {
            error!(
                %vm_id,
                socket = %socket_path.display(),
                "cloud-hypervisor did not create API socket within timeout"
            );
            let _ = child.kill().await;
            return Err(VmError::ProcessFailed(format!(
                "cloud-hypervisor did not create API socket at {} within 2s",
                socket_path.display()
            )));
        }

        // Detect premature exit — if the child is gone, no point waiting.
        if let Ok(Some(status)) = child.try_wait() {
            error!(%vm_id, ?status, "cloud-hypervisor exited before creating API socket");
            return Err(VmError::ProcessFailed(format!(
                "cloud-hypervisor exited with {status:?} before creating API socket"
            )));
        }

        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    debug!(%vm_id, pid, "cloud-hypervisor process spawned");

    Ok(child)
}

//TODO: need to add cleanup for the files and dir created
#[cfg(test)]
mod tests {
    use capnp::message::{Builder, HeapAllocator};
    use std::path::{Path, PathBuf};

    use crate::ch::dtos::CreateVmSpecRef;

    use super::{LocalArtifactResolver, create_vm_dir, prepare_artifacts};

    use std::sync::Once;
    use tracing_subscriber::{EnvFilter, fmt};

    static TRACING_INIT: Once = Once::new();

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
            cpus.set_max_vcpus(1);
        }

        {
            let mut memory = vm_cfg.reborrow().init_memory();
            memory.set_size(512 * 1024 * 1024);
        }

        {
            let mut payload = vm_cfg.reborrow().init_payload();
            payload.set_kernel(kernel);
            payload
                .set_cmdline("console=ttyS0 root=/dev/vda rw init=/nix/store/fake-toplevel/init");
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
        let runtime_dir = PathBuf::from("tests/data/tmp/sockets");

        let vm_id = "test-vm-001";
        let result = create_vm_dir(&runtime_dir, vm_id).await.unwrap();

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
}
