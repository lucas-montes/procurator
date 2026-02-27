//! VmManager — single owner of all VM state.
//!
//! Runs as one tokio task inside the Node. Owns a HashMap of VmHandles.
//! Generic over [`VmmBackend`] so the hypervisor layer can be swapped
//! for testing without touching real processes or sockets.
//!
//! Processes commands sequentially — no locks needed.

use std::collections::HashMap;
use std::path::PathBuf;

use tracing::{info, instrument, warn};

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
                let result = self.handle_create(spec).await.map(|_| CommandResponse::Unit);
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

    #[instrument(skip(self), fields(vm_id = %spec.id()))]
    async fn handle_create(&mut self, spec: VmSpec) -> Result<(), VmError> {
        if self.vms.contains_key(spec.id()) {
            return Err(VmError::AlreadyExists(spec.id().to_string()));
        }

        let vm_id = spec.id().to_string();
        info!(vm_id = %vm_id, store_path = %spec.store_path(), "Creating VM");

        // 1. Spawn the VMM process via the backend
        let (client, process, socket_path) = self.backend.spawn(&vm_id).await?;

        // 2. Build backend-specific config from the platform-agnostic spec
        let vmm_config = self.backend.build_config(&spec);

        // 3. Create the VM definition via the client
        client.create(vmm_config).await.map_err(|e| {
            VmError::Hypervisor(format!("vm.create failed: {e}"))
        })?;

        // 4. Boot the VM
        client.boot().await.map_err(|e| {
            VmError::Hypervisor(format!("vm.boot failed: {e}"))
        })?;

        // 5. Record in our table
        let handle = VmHandle {
            spec,
            client,
            process,
            socket_path,
            status: VmStatus::Running,
        };
        self.vms.insert(vm_id.clone(), handle);

        info!(vm_id = %vm_id, "VM created and booted");
        Ok(())
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
        VmInfo::new(
            vm_id.to_string(),
            self.config.worker_id.clone(),
            handle.status.clone(),
            handle.spec.content_hash().to_string(),
            handle.spec.content_hash().to_string(), // TODO: compute from running state
            VmMetrics::default(),
        )
    }
}
