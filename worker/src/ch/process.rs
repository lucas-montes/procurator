use std::path::PathBuf;

use futures::stream::TryStreamExt;

use rtnetlink;

use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

use crate::{vmm, vmm::Error as VmError};


pub struct Process {
    child: Child,
    socket_path: PathBuf,
    /// Per-VM working directory (contains writable disk copy, serial log, etc.)
    vm_dir: PathBuf,
    /// TAP device name owned by this VM. Deleted on cleanup via netlink.
    /// `None` when the VM was started without networking.
    tap_name: Option<String>,
}

impl vmm::Process for Process {
    async fn kill(&mut self) -> Result<(), VmError> {
        self.child
            .kill()
            .await
            .map_err(|e| VmError::ProcessFailed(format!("Failed to kill CH process: {e}")))
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, VmError> {
        self.child
            .try_wait()
            .map_err(|e| VmError::ProcessFailed(format!("Failed to check CH process: {e}")))
    }

    async fn cleanup(&mut self) -> Result<(), VmError> {
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
        if let Some(ref tap) = self.tap_name {
            match delete_tap_device(tap).await {
                Ok(()) => info!(tap = %tap, "TAP device deleted"),
                Err(e) => warn!(tap = %tap, error = %e, "Failed to delete TAP device"),
            }
        }

        if self.socket_path.exists() {
            let _ = tokio::fs::remove_file(&self.socket_path).await;
        }
        // Remove the entire per-VM working directory (writable disk, serial log, etc.)
        if self.vm_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&self.vm_dir).await;
        }
        Ok(())
    }
}

/// Delete a TAP device by name via netlink.
///
/// Requires `CAP_NET_ADMIN` — the worker process holds this via
/// systemd `AmbientCapabilities`.
async fn delete_tap_device(tap_name: &str) -> Result<(), VmError> {
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

/// Create a TAP device by name via `ioctl` on `/dev/net/tun`.
///
/// TAP devices are created through the TUN/TAP kernel interface, not via
/// netlink. The process:
///   1. `open("/dev/net/tun")`  — requires rw access (netdev group + DeviceAllow)
///   2. `ioctl(fd, TUNSETIFF, &ifreq)` — requires `CAP_NET_ADMIN`
///   3. `ioctl(fd, TUNSETPERSIST, 1)` — makes the TAP survive fd close
///
/// After creation, we use netlink to bring the interface up.
///
/// If the TAP already exists (e.g. from a previous crashed VM), it is
/// deleted first to avoid stale state.
async fn create_tap_device(tap_name: &str) -> Result<(), VmError> {
    // Delete stale TAP if it exists (crash recovery).
    // Best-effort — ignore errors if it doesn't exist.
    let _ = delete_tap_device(tap_name).await;

    // Create TAP via ioctl on /dev/net/tun.
    // This is a blocking syscall so we run it on the blocking pool.
    let name = tap_name.to_string();
    tokio::task::spawn_blocking(move || create_tap_ioctl(&name))
        .await
        .map_err(|e| VmError::Internal(format!("spawn_blocking for TAP creation panicked: {e}")))?
        .map_err(|e| VmError::Internal(format!("TAP ioctl creation failed: {e}")))?;

    // Bring the TAP up via netlink.
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
        .map_err(|e| VmError::Internal(format!("netlink get {tap_name} after create: {e}")))?
        .ok_or_else(|| VmError::Internal(format!("TAP {tap_name} not found after creation")))?;

    handle
        .link()
        .set(msg.header.index)
        .up()
        .execute()
        .await
        .map_err(|e| VmError::Internal(format!("netlink set {tap_name} up failed: {e}")))?;

    info!(tap = %tap_name, "TAP device created and brought up");
    Ok(())
}

/// Low-level TAP creation via `ioctl(2)`.
///
/// Opens `/dev/net/tun`, issues `TUNSETIFF` with `IFF_TAP | IFF_NO_PI`,
/// then `TUNSETPERSIST` so the device survives the fd being closed.
/// The fd is then dropped — CH will re-open the persistent TAP by name.
fn create_tap_ioctl(tap_name: &str) -> Result<(), std::io::Error> {
    use std::fs::OpenOptions;
    use std::os::unix::io::AsRawFd;

    // ioctl constants from <linux/if_tun.h>
    const TUNSETIFF: libc::c_ulong = 0x400454ca;
    const TUNSETPERSIST: libc::c_ulong = 0x400454cb;
    const IFF_TAP: libc::c_short = 0x0002;
    const IFF_NO_PI: libc::c_short = 0x1000;

    let tun_fd = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/net/tun")?;

    // Build ifreq struct — name + flags
    let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
    let name_bytes = tap_name.as_bytes();
    if name_bytes.len() >= libc::IFNAMSIZ {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "TAP name too long: {} (max {})",
                tap_name,
                libc::IFNAMSIZ - 1
            ),
        ));
    }
    // Copy name into ifr_name (null-terminated)
    unsafe {
        std::ptr::copy_nonoverlapping(
            name_bytes.as_ptr(),
            ifr.ifr_name.as_mut_ptr().cast::<u8>(),
            name_bytes.len(),
        );
    }
    ifr.ifr_ifru.ifru_flags = IFF_TAP | IFF_NO_PI;

    // TUNSETIFF — create the TAP device
    let ret = unsafe { libc::ioctl(tun_fd.as_raw_fd(), TUNSETIFF, &ifr) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // TUNSETPERSIST — keep the TAP alive after we close the fd.
    // CH will re-open it by name when it starts.
    let ret = unsafe { libc::ioctl(tun_fd.as_raw_fd(), TUNSETPERSIST, 1_i32) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // fd is dropped here — the persistent TAP remains in the kernel.
    Ok(())
}
