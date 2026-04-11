//! VMM abstraction layer for managing virtual machines.
//!
//! Three traits define the abstraction:
//!
//! - [`Client`] — per-VM client (one instance = one VM = one socket).
//!   Lifecycle operations: create, boot, shutdown, delete, pause, resume, etc.
//!
//! - [`Process`] — handle to the OS process backing one VM.
//!   Allows killing the process and cleaning up resources without knowing
//!   whether it's a real `tokio::process::Child` or a test stub.
//!
//! - [`Backend`] — factory that spawns VMM processes and creates clients.
//!   The VmManager is generic over this trait so it can be tested without
//!   touching real hypervisors, sockets, or the filesystem.

use std::fmt::Debug;

use super::{dtos::VmSpec, errors::Error};

/// One Vmm instance = one VM process = one socket.
pub trait Client {
    /// VMM-specific configuration type (e.g. VmConfig)
    type Config: Debug;
    /// VMM-specific error type
    type Error: std::error::Error;

    /// Create the VM definition (does NOT boot it)
    fn create(
        &self,
        config: Self::Config,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>>;

    /// Boot an already-created VM
    fn boot(&self) -> impl std::future::Future<Output = Result<(), Self::Error>>;

    /// Gracefully shut down the VM
    fn shutdown(&self) -> impl std::future::Future<Output = Result<(), Self::Error>>;

    /// Delete the VM definition (must be shut down first)
    fn delete(&self) -> impl std::future::Future<Output = Result<(), Self::Error>>;
}

/// Abstraction over the OS process that backs one VM.
///
/// Production: wraps `tokio::process::Child`.
/// Tests: a no-op stub that tracks calls.
pub trait Process {
    /// Kill the process. Best-effort — errors are logged, not propagated.
    fn kill(&mut self) -> impl std::future::Future<Output = Result<(), Error>>;

    /// Non-blocking check whether the process has exited.
    /// Returns `Ok(Some(status))` if exited, `Ok(None)` if still running.
    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, Error>;

    /// Clean up resources associated with this process (socket files, TAP
    /// devices, writable disk copies, etc.). Called after `kill`.
    fn cleanup(&mut self) -> impl std::future::Future<Output = Result<(), Error>>;
}

/// Factory that knows how to spawn VMM processes and build backend-specific
/// configs from a [`VmSpec`].
///
/// The VmManager is generic over this trait. In production the backend is
/// [`CloudHypervisorBackend`](super::cloud_hypervisor::CloudHypervisorBackend);
/// in tests it can be a mock that returns stub clients and processes.
pub trait Factory {
    /// The per-VM client this backend produces.
    type Client: Client;
    /// The process handle this backend produces.
    type Process: Process;

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
        spec: &VmSpec,
    ) -> impl std::future::Future<Output = Result<(), Error>>;

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
    ) -> impl std::future::Future<Output = Result<Vmm<Self::Client, Self::Process>, Error>>;

    /// Build a backend-specific VM config from the platform-agnostic [`VmSpec`].
    ///
    /// This is where Nix store-path → kernel/disk/initrd resolution happens.
    /// The `vm_id` is provided so the backend can look up per-VM prepared
    /// state (e.g. writable disk path from `prepare()`).
    fn build_config(&self, vm_id: &str, spec: &VmSpec) -> <Self::Client as Client>::Config;

    /// Attach the VM's network interface to the host network.
    ///
    /// Called between `client.create()` (hypervisor creates the TAP device)
    /// and `client.boot()`. This is where the TAP gets attached to the host
    /// bridge so the VM can reach the network.
    ///
    /// Default: no-op (for tests or backends without networking).
    fn attach_network(&self, vm_id: &str) -> impl std::future::Future<Output = Result<(), Error>>;
}

/// The main VMM manager struct that holds the client and process for one VM.
pub struct Vmm<C: Client, P: Process> {
    client: C,
    process: P,
    //TODO: it could hold the id
}

impl<C: Client, P: Process> Vmm<C, P> {
    pub fn new(client: C, process: P) -> Self {
        Self { client, process }
    }

    pub fn client(&self) -> &C {
        &self.client
    }

    pub fn process(&mut self) -> &mut P {
        &mut self.process
    }
}
