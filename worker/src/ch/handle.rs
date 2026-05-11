use std::path::PathBuf;

use tokio::process::Child;
use tracing::{debug, error, warn};

use crate::{
    ch::ip_allocator::IpAllocator,
    ch::tap::{Persisted, Tap},
    vmm::{Handle as VmHandle, HandleError},
};

use super::client::Client;

//TODO: I wonder if it makes sense this abstraction. The handler and the factory share similarities.
// The handler needs the ip state in ch and is part of the creation and cleanup. The handler needs a little bit too much state to be able to work,
// but hsould It be the case? I hope not and I don't really think so.
// The supervisor could know about all of this, however the creation of the child is being done with the factory + server side so we avoid
// serializing the whole message, and we can fail fast and send a message to the user. Then once we have the setup we want and the VM create,
// the boot process can be done in the background. The problem is that the IpAllocator is specific to ch, not to the overall logic,
// docker wouldn't need it, so it seems innapropriate to put it with the supervisor.
// Maybe we should rethink the Factory into a Proxy

pub struct Handle {
    /// The client to communicate with the cloud-hypervisor process.
    client: Client,
    /// The child process running cloud-hypervisor. We keep it to be able to kill it when needed.
    child: Child,
    /// Per-VM working directory (contains writable disk copy, serial log, etc.)
    vm_dir: PathBuf,
    /// Path to the writable root disk (inside `vm_dir`). Used for disk backups.
    writable_disk: PathBuf,
    /// The TAP interface used by the VM, we need it to clean up once we delete everything.
    tap: Tap<Persisted>,
    vm_id: String,
    lease_allocator: IpAllocator,
    ip: String,
}

impl Handle {
    pub fn new(
        client: Client,
        child: Child,
        vm_dir: PathBuf,
        writable_disk: PathBuf,
        tap: Tap<Persisted>,
        vm_id: String,
        lease_allocator: IpAllocator,
        ip: String,
    ) -> Self {
        Self {
            client,
            child,
            vm_dir,
            writable_disk,
            tap,
            vm_id,
            lease_allocator,
            ip,
        }
    }

    async fn cleanup(mut self) -> Result<(), HandleError> {
        debug!(vm_id = %self.vm_id, "Cleaning up VM");

        let socket_path = match self.client.kill().await {
            Ok(path) => Some(path),
            Err(err) => {
                error!(vm_id = %self.vm_id, error = %err, "Failed to kill CH process during cleanup");
                None
            }
        };

        self.tap
            .delete()
            .await
            .map_err(|e| HandleError::Cleanup(format!("Failed to delete TAP interface: {e}")))?;

        if let Err(err) = self.lease_allocator.release(&self.vm_id).await {
            warn!(vm_id = %self.vm_id, error = %err, "Failed to release IP lease");
        }

        if let Some(socket_path) = socket_path
            && socket_path.exists()
            && let Err(err) = tokio::fs::remove_file(&socket_path).await
        {
            error!(path = %socket_path.display(), error = %err, "Failed to remove CH socket file");
        }
        // Remove the entire per-VM working directory (writable disk, serial log, etc.)
        if self.vm_dir.exists()
            && let Err(err) = tokio::fs::remove_dir_all(&self.vm_dir).await
        {
            error!(path = %self.vm_dir.display(), error = %err, "Failed to remove CH self.vm_dir");
        }

        self.child
            .kill()
            .await
            .map_err(|e| HandleError::Cleanup(format!("Failed to kill CH process: {e}")))
    }
}

impl VmHandle for Handle {
    //TODO: maybe we could fetch if from the IpAllocator instead of saving it in the handle itself.
    fn ip(&self) -> &str {
        &self.ip
    }

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

    async fn pause(&self) -> Result<(), HandleError> {
        debug!(vm_id = %self.vm_id, "pausing VM");
        self.client
            .pause()
            .await
            .map_err(|e| HandleError::Backup(format!("pause VM: {e}")))
    }

    async fn resume(&self) -> Result<(), HandleError> {
        debug!(vm_id = %self.vm_id, "resuming VM");
        self.client
            .resume()
            .await
            .map_err(|e| HandleError::Backup(format!("resume VM: {e}")))
    }

    async fn snapshot(&self, destination: PathBuf) -> Result<(), HandleError> {
        debug!(vm_id = %self.vm_id, dest = %destination.display(), "snapshotting VM");

        tokio::fs::create_dir_all(&destination)
            .await
            .map_err(|e| HandleError::Backup(format!("create snapshot dir: {e}")))?;

        // CH expects a file:// URL with a trailing slash pointing at an existing directory.
        let dest_str = destination
            .to_str()
            .ok_or_else(|| HandleError::Backup("snapshot destination is not valid UTF-8".into()))?;
        let url = format!("file://{}/", dest_str.trim_end_matches('/'));

        self.client
            .pause()
            .await
            .map_err(|e| HandleError::Backup(format!("pause VM: {e}")))?;

        let snap_res = self.client.snapshot(&url).await;

        // Always try to resume, even if the snapshot itself failed.
        let resume_res = self.client.resume().await;

        snap_res.map_err(|e| HandleError::Backup(format!("snapshot VM: {e}")))?;
        resume_res.map_err(|e| HandleError::Backup(format!("resume VM after snapshot: {e}")))?;
        Ok(())
    }

    async fn backup_disk(&self, destination: PathBuf) -> Result<(), HandleError> {
        debug!(
            vm_id = %self.vm_id,
            src = %self.writable_disk.display(),
            dest = %destination.display(),
            "backing up VM disk"
        );

        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| HandleError::Backup(format!("create backup parent dir: {e}")))?;
        }

        self.client
            .pause()
            .await
            .map_err(|e| HandleError::Backup(format!("pause VM: {e}")))?;

        // tokio::fs::copy uses std::fs::copy under the hood, which on Linux already tries
        // copy_file_range (and thus reflink on btrfs/xfs/zfs) before falling back to a
        // userspace read/write loop.
        let copy_res = tokio::fs::copy(&self.writable_disk, &destination).await;

        let resume_res = self.client.resume().await;

        copy_res.map_err(|e| HandleError::Backup(format!("copy disk: {e}")))?;
        resume_res.map_err(|e| HandleError::Backup(format!("resume VM after backup: {e}")))?;

        Ok(())
    }
}
