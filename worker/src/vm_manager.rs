//! # VmManager — single owner of all VM state (ADR-002, ADR-005)
//!
//! Fleet manager: owns `HashMap<VmId, VmHandle>`, processes commands sequentially
//! in one tokio task. No locks — state mutations happen in one place, in order.
//!
//! ## vs [`vmm`](crate::vmm)
//!
//! `vmm` = driver for **one** hypervisor process (REST client + OS process handle).
//! `VmManager` = dispatcher that owns **N** drivers and routes commands to them.
//!
//! ## Design
//!
//! - **Single owner** — only `VmManager` mutates the VM table. No `Arc<Mutex<_>>`.
//! - **Message passing** — Server sends `CommandPayload` over mpsc, awaits oneshot reply.
//! - **Generic over `VmmBackend`** — production uses `CloudHypervisorBackend`, tests use `MockBackend`.
//!
//! ## Create flow
//!
//! UUIDv7 → `prepare(vm_id, spec)` → `spawn(vm_id)` → `build_config(vm_id, spec)`
//! → `client.create(config)` → `client.boot()` → insert `VmHandle`.
//! On failure, no `VmHandle` is inserted — no partial state.
//!
//! ## Delete flow
//!
//! Remove from HashMap → `shutdown()` (best-effort) → `delete()` (best-effort)
//! → `kill()` → `cleanup()` (socket, disk copy, serial log, VM dir).

use std::collections::HashMap;
use std::path::PathBuf;

use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use crate::dto::{
    CommandPayload, CommandResponse, Message, VmError, VmInfo,
    VmMetrics, VmSpec, VmStatus, WorkerInfo,
};
use crate::vmm::{Vmm, VmmBackend, VmmProcess};

// ─── Per-VM state ──────────────────────────────────────────────────────────

/// Everything the manager knows about one VM.
///
/// Generic over `B: VmmBackend` so the client and process types come from
/// the backend, not hardcoded to cloud-hypervisor.
struct VmHandle<B: VmmBackend> {
    /// The spec that created this VM
    spec: VmSpec,
    /// Per-VM client (e.g. CH REST client)
    client: B::Client,
    /// OS process handle (e.g. CH child process)
    process: B::Process,
    /// Path to the API socket (for reference / cleanup)
    socket_path: PathBuf,
    /// Current observed status
    status: VmStatus,
}

// ─── Configuration ─────────────────────────────────────────────────────────

/// Runtime configuration for the VmManager.
///
/// Backend-specific config (socket dirs, binary paths, timeouts) lives
/// in the backend's own config type (e.g. `CloudHypervisorConfig`).
pub struct VmManagerConfig {
    /// Worker identity string
    pub worker_id: String,
}

impl Default for VmManagerConfig {
    fn default() -> Self {
        Self {
            worker_id: String::from("worker-local"),
        }
    }
}

// ─── VmManager ─────────────────────────────────────────────────────────────

pub struct VmManager<B: VmmBackend> {
    vms: HashMap<String, VmHandle<B>>,
    config: VmManagerConfig,
    backend: B,
}

impl<B: VmmBackend> VmManager<B> {
    pub fn new(backend: B, config: VmManagerConfig) -> Self {
        Self {
            vms: HashMap::new(),
            config,
            backend,
        }
    }

    /// Dispatch a command to the appropriate handler and send the reply.
    /// Called by Node's recv loop.
    pub async fn handle(&mut self, msg: Message) {
        let (data, reply) = msg.into_parts();
        match data {
            CommandPayload::Create(spec) => {
                let result = self.handle_create(spec).await.map(CommandResponse::VmId);
                let _ = reply.send(result);
            }
            CommandPayload::Delete(vm_id) => {
                let result = self
                    .handle_delete(&vm_id)
                    .await
                    .map(|_| CommandResponse::Unit);
                let _ = reply.send(result);
            }
            CommandPayload::List => {
                let result = self.handle_list().await.map(CommandResponse::VmList);
                let _ = reply.send(result);
            }
            CommandPayload::GetWorkerStatus => {
                let result = self
                    .handle_get_worker_status()
                    .await
                    .map(CommandResponse::WorkerInfo);
                let _ = reply.send(result);
            }
        }
    }

    // ─── Command handlers ──────────────────────────────────────────────

    #[instrument(skip(self), fields(toplevel = %spec.toplevel()))]
    async fn handle_create(&mut self, spec: VmSpec) -> Result<String, VmError> {
        let vm_id = Uuid::now_v7().to_string();
        info!(
            vm_id = %vm_id,
            toplevel = %spec.toplevel(),
            kernel = %spec.kernel_path(),
            disk = %spec.disk_image_path(),
            cpu = spec.cpu(),
            memory_mb = spec.memory_mb(),
            "Creating VM"
        );

        // 1. Ensure artifacts are available locally (e.g. nix copy from cache)
        //    Also copies the disk image to a writable location for this VM.
        self.backend.prepare(&vm_id, &spec).await?;
        info!(vm_id = %vm_id, "prepare complete");

        // 2. Spawn the VMM process via the backend
        let (client, mut process, socket_path) = self.backend.spawn(&vm_id).await?;
        info!(vm_id = %vm_id, socket = %socket_path.display(), "VMM process spawned");

        // 3. Build backend-specific config from the platform-agnostic spec
        //    Uses the writable disk path created by prepare().
        let vmm_config = self.backend.build_config(&vm_id, &spec);

        // 4. Create the VM definition via the client
        client.create(vmm_config).await.map_err(|e| {
            VmError::Hypervisor(format!("vm.create failed: {e}"))
        })?;

        // 5. Boot the VM
        client.boot().await.map_err(|e| {
            VmError::Hypervisor(format!("vm.boot failed: {e}"))
        })?;

        // 6. Quick liveness check — did CH crash right after boot?
        //    Give it a moment, then verify the process is still alive.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        match process.try_wait() {
            Ok(Some(exit_status)) => {
                error!(
                    vm_id = %vm_id,
                    exit_status = %exit_status,
                    "VMM process exited immediately after boot"
                );
                // Read the CH log before we clean up
                if let Err(e) = process.cleanup().await {
                    warn!(vm_id = %vm_id, error = ?e, "cleanup after crash failed");
                }
                return Err(VmError::ProcessFailed(format!(
                    "VMM process exited with {exit_status} immediately after boot"
                )));
            }
            Ok(None) => {
                info!(vm_id = %vm_id, "VMM process alive after boot");
            }
            Err(e) => {
                warn!(vm_id = %vm_id, error = %e, "could not check VMM process status");
            }
        }

        // 7. Record in our table
        let handle = VmHandle {
            spec,
            client,
            process,
            socket_path,
            status: VmStatus::Running,
        };
        self.vms.insert(vm_id.clone(), handle);

        info!(vm_id = %vm_id, "VM created and booted successfully");
        Ok(vm_id)
    }

    #[instrument(skip(self))]
    async fn handle_delete(&mut self, vm_id: &str) -> Result<(), VmError> {
        let mut handle = self
            .vms
            .remove(vm_id)
            .ok_or_else(|| VmError::NotFound(vm_id.to_string()))?;

        info!(vm_id = %vm_id, "Deleting VM");

        // Try graceful shutdown, ignore errors (may already be stopped)
        if let Err(e) = handle.client.shutdown().await {
            warn!(vm_id = %vm_id, error = ?e, "Shutdown failed (may already be stopped)");
        }

        // Delete VM definition
        if let Err(e) = handle.client.delete().await {
            warn!(vm_id = %vm_id, error = ?e, "Delete failed");
        }

        // Kill the process and clean up resources
        if let Err(e) = handle.process.kill().await {
            warn!(vm_id = %vm_id, error = ?e, "Failed to kill VMM process");
        }
        if let Err(e) = handle.process.cleanup().await {
            warn!(vm_id = %vm_id, error = ?e, "Cleanup failed");
        }

        info!(vm_id = %vm_id, "VM deleted");
        Ok(())
    }

    async fn handle_list(&self) -> Result<Vec<VmInfo>, VmError> {
        let infos = self
            .vms
            .iter()
            .map(|(id, handle)| self.build_vm_info(id, handle))
            .collect();
        Ok(infos)
    }

    async fn handle_get_worker_status(&self) -> Result<WorkerInfo, VmError> {
        let running = self
            .vms
            .values()
            .filter(|h| matches!(h.status, VmStatus::Running))
            .count() as u32;

        Ok(WorkerInfo::new(
            self.config.worker_id.clone(),
            true,
            0,
            running,
        ))
    }

    // ─── Helpers ───────────────────────────────────────────────────────

    fn build_vm_info(&self, vm_id: &str, handle: &VmHandle<B>) -> VmInfo {
        let toplevel_hash = handle.spec.toplevel().to_string();
        VmInfo::new(
            vm_id.to_string(),
            self.config.worker_id.clone(),
            handle.status.clone(),
            toplevel_hash.clone(),
            toplevel_hash, // TODO: compute from running state
            VmMetrics::default(),
        )
    }
}
