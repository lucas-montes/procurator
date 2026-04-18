use std::path::PathBuf;

use tokio::process::Child;
use tracing::{debug, error, warn};

use crate::{
    ch::tap::Tap,
    vmm::{Handle as VmHandle, HandleError},
};

use super::client::Client;

pub struct Handle {
    client: Client,
    child: Child,
    /// Per-VM working directory (contains writable disk copy, serial log, etc.)
    vm_dir: PathBuf,
    //TODO: not sure if we want to keep it here or we want to make it persistent and just pass the name around
    _tap: Tap,
}

impl Handle {
    pub fn new(client: Client, child: Child, vm_dir: PathBuf, tap: Tap) -> Self {
        Self {
            client,
            child,
            vm_dir,
            _tap: tap,
        }
    }

    async fn kill(&mut self) -> Result<(), HandleError> {
        self.child
            .kill()
            .await
            .map_err(|e| HandleError::Cleanup(format!("Failed to kill CH process: {e}")))
    }

    async fn cleanup(self) -> Result<(), HandleError> {
        // TODO: not sure this makes any sense Log CH output for post-mortem debugging before cleaning up.
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
        // if let Some(ref tap) = self.tap_name {
        //     match delete_tap_device(tap).await {
        //         Ok(()) => info!(tap = %tap, "TAP device deleted"),
        //         Err(e) => warn!(tap = %tap, error = %e, "Failed to delete TAP device"),
        //     }
        // }

        let socket_path = self
            .client
            .kill()
            .await
            .map_err(|e| HandleError::Cleanup(e.to_string()))?;
        if socket_path.exists() {
            if let Err(err) = tokio::fs::remove_file(&socket_path).await {
                error!(path = %socket_path.display(), error = %err, "Failed to remove CH socket file");
            };
        }
        // Remove the entire per-VM working directory (writable disk, serial log, etc.)
        if self.vm_dir.exists() {
            if let Err(err) = tokio::fs::remove_file(&self.vm_dir).await {
                error!(path = %self.vm_dir.display(), error = %err, "Failed to remove CH self.vm_dir");
            };
        }
        Ok(())
    }
}

impl VmHandle for Handle {
    async fn start(&self) -> Result<(), HandleError> {
        self.client
            .boot()
            .await
            .map_err(|e| HandleError::Start(format!("Failed to boot VM: {e}")))
    }

    /// As we take ownership of the handle itself everything inside of it should be dropped, meaning that the TAP interface should be deleted, no need to remove the tap manually
    /// at least for now. If we ever use `persist` we'll need to cleanup the tap interface to be able to reuse it.
    async fn delete(mut self) -> Result<(), HandleError> {
        self.kill()
            .await
            .map_err(|e| HandleError::Delete(format!("Failed to kill VM: {e}")))?;
        self.cleanup()
            .await
            .map_err(|e| HandleError::Cleanup(format!("Failed to cleanup VM: {e}")))
    }

    async fn health(&self) -> Result<(), HandleError> {
        //TODO: create the funcion in the client to get stats and info
        Ok(())
    }
}
