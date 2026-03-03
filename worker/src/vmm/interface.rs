//! VMM abstraction layer for managing virtual machines.
//!
//! Three traits define the abstraction:
//!
//! - [`Vmm`] — per-VM client (one instance = one VM = one socket).
//!   Lifecycle operations: create, boot, shutdown, delete, pause, resume, etc.
//!
//! - [`VmmProcess`] — handle to the OS process backing one VM.
//!   Allows killing the process and cleaning up resources without knowing
//!   whether it's a real `tokio::process::Child` or a test stub.
//!
//! - [`VmmBackend`] — factory that spawns VMM processes and creates clients.
//!   The VmManager is generic over this trait so it can be tested without
//!   touching real hypervisors, sockets, or the filesystem.

use std::fmt::Debug;
use std::path::PathBuf;

use crate::dto::{VmError, VmSpec};

// ─── Per-VM client ─────────────────────────────────────────────────────────

/// One Vmm instance = one VM process = one socket. The multi-VM layer
/// is VmManager, not this trait.
pub trait Vmm: Send + 'static {
    /// VMM-specific configuration type (e.g. ChVmConfig)
    type Config: Debug + Send;
    /// VMM-specific info type (e.g. ChVmInfo)
    type Info: Debug + Send;
    /// VMM-specific error type
    type Error: std::error::Error + Send;

    /// Create the VM definition (does NOT boot it)
    fn create(
        &self,
        config: Self::Config,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    /// Boot an already-created VM
    fn boot(&self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    /// Gracefully shut down the VM
    fn shutdown(&self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    /// Delete the VM definition (must be shut down first)
    fn delete(&self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    /// Get information about this VM
    fn info(&self) -> impl std::future::Future<Output = Result<Self::Info, Self::Error>> + Send;

    /// Pause the VM (freeze vCPUs)
    fn pause(&self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    /// Resume a paused VM
    fn resume(&self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    /// Get I/O counters (network/disk) for metrics
    fn counters(
        &self,
    ) -> impl std::future::Future<Output = Result<Self::Info, Self::Error>> + Send;

    /// Ping the VMM process to check if it's alive
    fn ping(&self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;
}

// ─── VMM process handle ───────────────────────────────────────────────────

/// Abstraction over the OS process that backs one VM.
///
/// Production: wraps `tokio::process::Child`.
/// Tests: a no-op stub that tracks calls.
pub trait VmmProcess: Send + 'static {
    /// Kill the process. Best-effort — errors are logged, not propagated.
    fn kill(&mut self) -> impl std::future::Future<Output = Result<(), VmError>> + Send;

    /// Non-blocking check whether the process has exited.
    /// Returns `Ok(Some(status))` if exited, `Ok(None)` if still running.
    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, VmError>;

    /// Clean up resources associated with this process (socket files, TAP
    /// devices, writable disk copies, etc.). Called after `kill`.
    fn cleanup(&mut self) -> impl std::future::Future<Output = Result<(), VmError>> + Send;
}

// ─── Backend factory ──────────────────────────────────────────────────────

/// Factory that knows how to spawn VMM processes and build backend-specific
/// configs from a [`VmSpec`].
///
/// The VmManager is generic over this trait. In production the backend is
/// [`CloudHypervisorBackend`](super::cloud_hypervisor::CloudHypervisorBackend);
/// in tests it can be a mock that returns stub clients and processes.
pub trait VmmBackend: Send + 'static {
    /// The per-VM client this backend produces.
    type Client: Vmm;
    /// The process handle this backend produces.
    type Process: VmmProcess;

    /// Ensure the VM's artifacts (kernel, disk image, initrd) are available
    /// on the local filesystem before spawning.
    ///
    /// Responsibilities:
    /// - Validate that store paths (kernel, initrd, disk) exist locally
    /// - Copy the disk image to a writable location for the VM
    /// - Create per-VM directories (for serial logs, writable disk, etc.)
    ///
    /// Production: validates paths, copies disk to `/tmp/procurator/vms/{vm_id}/disk.img`.
    /// Future: may run `nix copy --from <cache> <store-path>` before validating.
    /// Tests: no-op (paths don't need to exist).
    ///
    /// The `vm_id` is provided so the backend can create per-VM directories
    /// and store prepared state (writable disk path, serial log path) that
    /// `build_config` and `cleanup` will use later.
    fn prepare(
        &self,
        vm_id: &str,
        _spec: &VmSpec,
    ) -> impl std::future::Future<Output = Result<(), VmError>> + Send {
        let _ = vm_id;
        std::future::ready(Ok(()))
    }

    /// Spawn a new VMM process for the given VM and return a client + process handle.
    ///
    /// Responsibilities (for a real backend):
    /// - ensure directories exist
    /// - spawn the hypervisor process
    /// - wait for the API socket to become ready
    /// - construct the per-VM client
    fn spawn(
        &self,
        vm_id: &str,
    ) -> impl std::future::Future<Output = Result<(Self::Client, Self::Process, PathBuf), VmError>> + Send;

    /// Build a backend-specific VM config from the platform-agnostic [`VmSpec`].
    ///
    /// This is where Nix store-path → kernel/disk/initrd resolution happens.
    /// The `vm_id` is provided so the backend can look up per-VM prepared
    /// state (e.g. writable disk path from `prepare()`).
    fn build_config(&self, vm_id: &str, spec: &VmSpec) -> <Self::Client as Vmm>::Config;
}
