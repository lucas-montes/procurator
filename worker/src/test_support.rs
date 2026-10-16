//! Test-only fakes: fake VMM backend plus temp sqlite databases.
//!
//! Never compiled into the real binary (module is `#[cfg(test)]`). Shared by
//! config and pull-loop tests so both speak the same fake backend.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Deserialize;

use crate::{
    database::Database,
    vmm::{CreateCommand, Error as VmmError, Factory, Handle, HandleError},
};

/// Fake backend config: empty, deserializes from `{}`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FakeConfig {}

/// Fake creation spec: wire content is irrelevant (keys come from the poll
/// index until plan T09 adds real ids), so conversion always succeeds.
#[derive(Debug, Clone)]
pub struct FakeSpec;

/// Fake VM handle with programmable health (T06 heartbeat tests reuse this).
#[derive(Debug)]
pub struct FakeHandle {
    ip: String,
    healthy: bool,
}

impl FakeHandle {
    /// Handle reporting healthy.
    pub fn healthy() -> Self {
        Self {
            ip: "10.0.0.99".to_string(),
            healthy: true,
        }
    }

    /// Handle reporting failed health (heartbeat tests).
    pub fn failing() -> Self {
        Self {
            ip: "10.0.0.99".to_string(),
            healthy: false,
        }
    }
}

impl Handle for FakeHandle {
    fn ip(&self) -> &str {
        &self.ip
    }

    async fn start(&self) -> Result<(), HandleError> {
        Ok(())
    }

    async fn delete(self) -> Result<(), HandleError> {
        Ok(())
    }

    async fn health(&self) -> Result<(), HandleError> {
        if self.healthy {
            Ok(())
        } else {
            Err(HandleError::Start("fake unhealthy".to_string()))
        }
    }

    async fn pause(&self) -> Result<(), HandleError> {
        Ok(())
    }

    async fn resume(&self) -> Result<(), HandleError> {
        Ok(())
    }

    async fn snapshot(&self, _destination: PathBuf) -> Result<(), HandleError> {
        Ok(())
    }

    async fn backup_disk(&self, _destination: PathBuf) -> Result<(), HandleError> {
        Ok(())
    }
}

/// Fake factory: infallible creation, [`Handle`]s never touch a hypervisor.
#[derive(Debug, Clone, Default)]
pub struct FakeFactory;

impl Factory for FakeFactory {
    type VmHandle = FakeHandle;
    type Config = FakeConfig;
    type BackendConfig = capnp::any_pointer::Owned;
    type CreateVmSpec<'a> = FakeSpec;

    fn create_id() -> String {
        "fake-id".to_string()
    }

    async fn create_vm(
        &self,
        _source: Self::CreateVmSpec<'_>,
    ) -> Result<CreateCommand<Self>, VmmError> {
        Ok(CreateCommand::new(FakeHandle::healthy(), Self::create_id()))
    }
}

impl<'a> TryFrom<commands::common_capnp::vm_spec::Reader<'a, capnp::any_pointer::Owned>>
    for FakeSpec
{
    type Error = capnp::Error;

    fn try_from(
        _: commands::common_capnp::vm_spec::Reader<'a, capnp::any_pointer::Owned>,
    ) -> Result<Self, Self::Error> {
        Ok(FakeSpec)
    }
}

/// Failing factory: creation always errors (exercises reconcile error paths).
#[derive(Debug, Clone, Default)]
pub struct FailFactory;

impl Factory for FailFactory {
    type VmHandle = FakeHandle;
    type Config = FakeConfig;
    type BackendConfig = capnp::any_pointer::Owned;
    type CreateVmSpec<'a> = FakeSpec;

    fn create_id() -> String {
        "fake-id".to_string()
    }

    async fn create_vm(
        &self,
        _source: Self::CreateVmSpec<'_>,
    ) -> Result<CreateCommand<Self>, VmmError> {
        Err(VmmError::Internal("fake creation failure".to_string()))
    }
}

/// Temp sqlite database file, removed on drop (with WAL sidecars).
pub struct TempDb {
    path: PathBuf,
}

impl TempDb {
    /// Creates a fresh migrated database at a unique temp path.
    pub async fn create(name: &str) -> (Database, Self) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "procurator-{name}-{}-{}.db",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let url = format!("sqlite:{}", path.display());
        let db = Database::new(&url).await;
        (db, Self { path })
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut path = self.path.as_os_str().to_owned();
            path.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(path));
        }
    }
}
