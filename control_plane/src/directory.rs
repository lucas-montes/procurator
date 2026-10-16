//! Worker capability directory: swappable seam for worker RPC clients.
//!
//! The pull-mode control plane never dials workers, so v1
//! ([`NoWorkerDirectory`]) always reports capabilities unavailable. A future
//! push transport injects real clients via `Server::with_directory` without
//! touching `Server`.
//!
//! The capability type is fixed to `any_pointer::Owned`: the generated
//! `get_worker_results` setter is branded that way regardless of the
//! interface's `BackendConfig`, and capabilities are untyped on the wire, so
//! erasing the backend parameter at this boundary is sound.

use std::fmt::{Display, Formatter};

use commands::worker_capnp::worker::Client as WorkerClient;

use crate::registry::WorkerId;

/// Resolves a worker id to a live `Worker` RPC capability.
///
/// Implementations run thread-locally next to the single-threaded RPC loop
/// (capability clients are not `Send`), so no `Send`/`Sync` bounds are
/// required.
pub trait WorkerDirectory {
    /// Looks up the live capability for `worker_id`.
    ///
    /// # Errors
    ///
    /// Returns [`DirectoryError::Unavailable`] when no route exists (always,
    /// with [`NoWorkerDirectory`]).
    fn lookup(
        &self,
        worker_id: &WorkerId,
    ) -> Result<WorkerClient<capnp::any_pointer::Owned>, DirectoryError>;
}

/// v1 directory: pull mode never dials workers.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoWorkerDirectory;

impl WorkerDirectory for NoWorkerDirectory {
    fn lookup(
        &self,
        worker_id: &WorkerId,
    ) -> Result<WorkerClient<capnp::any_pointer::Owned>, DirectoryError> {
        Err(DirectoryError::Unavailable(worker_id.clone()))
    }
}

/// Capability resolution failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectoryError {
    /// No route to a live worker capability exists.
    Unavailable(WorkerId),
}

impl Display for DirectoryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(id) => write!(
                f,
                "no worker capability for '{id}': control plane runs pull-only and never dials workers (push transport follow-up)"
            ),
        }
    }
}

impl std::error::Error for DirectoryError {}

#[cfg(test)]
mod tests {
    use super::{NoWorkerDirectory, WorkerDirectory as _};
    use crate::registry::WorkerId;

    #[test]
    fn pull_mode_directory_is_always_unavailable() {
        let directory = NoWorkerDirectory;
        match directory.lookup(&WorkerId::new("w1").expect("valid")) {
            Err(err) => {
                let message = format!("{err}");
                assert!(message.contains("w1"), "unexpected error: {message}");
                assert!(message.contains("pull-only"), "unexpected error: {message}");
            }
            Ok(_) => panic!("pull mode must not resolve capabilities"),
        }
    }
}
