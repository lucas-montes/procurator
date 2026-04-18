use std::path::PathBuf;

use tokio::process::Child;
use tracing::{debug, error, warn};

use crate::{
    ch::tap::{Persisted, Tap},
    vmm::{Handle as VmHandle, HandleError},
};

use super::client::Client;

pub struct Handle {
    client: Client,
    child: Child,
    /// Per-VM working directory (contains writable disk copy, serial log, etc.)
    vm_dir: PathBuf,
    tap: Tap<Persisted>,
}

impl Handle {
    pub fn new(client: Client, child: Child, vm_dir: PathBuf, tap: Tap<Persisted>) -> Self {
        Self {
            client,
            child,
            vm_dir,
            tap,
        }
    }

    async fn cleanup(mut self) -> Result<(), HandleError> {
        let socket_path = self
            .client
            .kill()
            .await
            .map_err(|e| HandleError::Cleanup(e.to_string()))?;

        self.tap
            .delete()
            .await
            .map_err(|e| HandleError::Cleanup(format!("Failed to delete TAP interface: {e}")))?;

        if socket_path.exists() {
            if let Err(err) = tokio::fs::remove_file(&socket_path).await {
                error!(path = %socket_path.display(), error = %err, "Failed to remove CH socket file");
            };
        }
        // Remove the entire per-VM working directory (writable disk, serial log, etc.)
        if self.vm_dir.exists() {
            if let Err(err) = tokio::fs::remove_dir_all(&self.vm_dir).await {
                error!(path = %self.vm_dir.display(), error = %err, "Failed to remove CH self.vm_dir");
            };
        }

        self.child
            .kill()
            .await
            .map_err(|e| HandleError::Cleanup(format!("Failed to kill CH process: {e}")))
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
    async fn delete(self) -> Result<(), HandleError> {
        self.cleanup()
            .await
            .map_err(|e| HandleError::Cleanup(format!("Failed to cleanup VM: {e}")))
    }

    async fn health(&self) -> Result<(), HandleError> {
        //TODO: create the funcion in the client to get stats and info
        Ok(())
    }
}
