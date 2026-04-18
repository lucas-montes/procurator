use std::path::PathBuf;

use futures::stream::TryStreamExt;

use tokio::process::Child;
use tracing::{debug, warn};

use crate::{
    ch::tap::Tap,
    vmm::{Error as VmError, Handle as VmHandle},
};

use super::client::Client;

pub struct Handle {
    client: Client,
    child: Child,
    /// Per-VM working directory (contains writable disk copy, serial log, etc.)
    vm_dir: PathBuf,
    //TODO: not sure if we want to keep it here or we want to make it persistent and just pass the name around
    tap: Tap,
}

impl Handle {
    pub fn new(client: Client, child: Child, vm_dir: PathBuf, tap: Tap) -> Self {
        Self {
            client,
            child,
            vm_dir,
            tap,
        }
    }

    pub async fn kill(&mut self) -> Result<(), VmError> {
        self.child
            .kill()
            .await
            .map_err(|e| VmError::ProcessFailed(format!("Failed to kill CH process: {e}")))
    }

    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, VmError> {
        self.child
            .try_wait()
            .map_err(|e| VmError::ProcessFailed(format!("Failed to check CH process: {e}")))
    }

    pub async fn cleanup(&mut self) -> Result<(), VmError> {
        // Log CH output for post-mortem debugging before cleaning up.
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

        // TODO: the client holds the socketPath
        // if self.socket_path.exists() {
        // let _ = tokio::fs::remove_file(&self.socket_path).await;
        // }
        // Remove the entire per-VM working directory (writable disk, serial log, etc.)
        if self.vm_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&self.vm_dir).await;
        }
        Ok(())
    }
}

impl VmHandle for Handle {}

/// Delete a TAP device by name via netlink.
///
/// Requires `CAP_NET_ADMIN` — the worker process holds this via
/// systemd `AmbientCapabilities`.
pub(crate) async fn delete_tap_device(tap_name: &str) -> Result<(), VmError> {
    let (connection, handle, _) = rtnetlink::new_connection()
        .map_err(|e| VmError::Internal(format!("netlink connection failed: {e}")))?;
    tokio::spawn(connection);

    let mut links = handle
        .link()
        .get()
        .match_name(tap_name.to_string())
        .execute();
    let msg = links
        .try_next()
        .await
        .map_err(|e| VmError::Internal(format!("netlink get {tap_name} failed: {e}")))?;

    if let Some(link) = msg {
        handle
            .link()
            .del(link.header.index)
            .execute()
            .await
            .map_err(|e| VmError::Internal(format!("netlink del {tap_name} failed: {e}")))?;
    }
    Ok(())
}
