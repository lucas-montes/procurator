//! Mock VMM backend for unit testing.
//!
//! Provides [`MockBackend`], [`MockVmm`], and [`MockProcess`] — lightweight
//! implementations of the VMM traits that track calls without touching real
//! processes, sockets, or the filesystem.
//!
//! Each mock records what was called so tests can assert on the sequence
//! of operations. Failures can be injected via [`MockBackendConfig`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::dto::{VmError, VmSpec};
use crate::vmm::{Vmm, VmmBackend, VmmProcess};

// ─── Configuration for failure injection ──────────────────────────────────

/// Controls which operations should fail in the mock.
/// All default to `None` (success).
#[derive(Debug, Clone, Default)]
pub struct MockBackendConfig {
    /// If set, `prepare()` returns this error
    pub prepare_error: Option<String>,
    /// If set, `spawn()` returns this error
    pub spawn_error: Option<String>,
    /// If set, `Vmm::create()` returns an error
    pub create_error: Option<String>,
    /// If set, `Vmm::boot()` returns an error
    pub boot_error: Option<String>,
    /// If set, `Vmm::shutdown()` returns an error
    pub shutdown_error: Option<String>,
    /// If set, `Vmm::delete()` returns an error
    pub delete_error: Option<String>,
}

// ─── Call tracker (shared between backend, client, process) ───────────────

/// Shared counter tracking how many VMs have been spawned.
/// Tests use this to verify the expected number of spawn calls.
#[derive(Debug, Clone, Default)]
pub struct MockCallTracker {
    pub prepares: Arc<AtomicUsize>,
    pub spawns: Arc<AtomicUsize>,
    pub creates: Arc<AtomicUsize>,
    pub boots: Arc<AtomicUsize>,
    pub shutdowns: Arc<AtomicUsize>,
    pub deletes: Arc<AtomicUsize>,
    pub kills: Arc<AtomicUsize>,
    pub cleanups: Arc<AtomicUsize>,
}

impl MockCallTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn prepare_count(&self) -> usize {
        self.prepares.load(Ordering::Relaxed)
    }

    pub fn spawn_count(&self) -> usize {
        self.spawns.load(Ordering::Relaxed)
    }

    pub fn create_count(&self) -> usize {
        self.creates.load(Ordering::Relaxed)
    }

    pub fn boot_count(&self) -> usize {
        self.boots.load(Ordering::Relaxed)
    }

    pub fn shutdown_count(&self) -> usize {
        self.shutdowns.load(Ordering::Relaxed)
    }

    pub fn delete_count(&self) -> usize {
        self.deletes.load(Ordering::Relaxed)
    }

    pub fn kill_count(&self) -> usize {
        self.kills.load(Ordering::Relaxed)
    }

    pub fn cleanup_count(&self) -> usize {
        self.cleanups.load(Ordering::Relaxed)
    }
}

// ─── Mock VMM client ──────────────────────────────────────────────────────

/// Mock per-VM client. Records calls and optionally returns errors.
pub struct MockVmm {
    tracker: MockCallTracker,
    config: MockBackendConfig,
}

/// Config type for MockVmm (just the VmSpec fields, for assertions).
#[derive(Debug)]
pub struct MockVmConfig {
    pub cpu: u32,
    pub memory_mb: u32,
    pub kernel_path: String,
    pub disk_image_path: String,
}

/// Info type for MockVmm.
#[derive(Debug)]
pub struct MockVmInfo {
    pub state: String,
}

/// Error type for MockVmm.
#[derive(Debug)]
pub struct MockVmError(pub String);

impl std::fmt::Display for MockVmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mock error: {}", self.0)
    }
}

impl std::error::Error for MockVmError {}

impl Vmm for MockVmm {
    type Config = MockVmConfig;
    type Info = MockVmInfo;
    type Error = MockVmError;

    async fn create(&self, _config: Self::Config) -> Result<(), Self::Error> {
        self.tracker.creates.fetch_add(1, Ordering::Relaxed);
        if let Some(ref e) = self.config.create_error {
            return Err(MockVmError(e.clone()));
        }
        Ok(())
    }

    async fn boot(&self) -> Result<(), Self::Error> {
        self.tracker.boots.fetch_add(1, Ordering::Relaxed);
        if let Some(ref e) = self.config.boot_error {
            return Err(MockVmError(e.clone()));
        }
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), Self::Error> {
        self.tracker.shutdowns.fetch_add(1, Ordering::Relaxed);
        if let Some(ref e) = self.config.shutdown_error {
            return Err(MockVmError(e.clone()));
        }
        Ok(())
    }

    async fn delete(&self) -> Result<(), Self::Error> {
        self.tracker.deletes.fetch_add(1, Ordering::Relaxed);
        if let Some(ref e) = self.config.delete_error {
            return Err(MockVmError(e.clone()));
        }
        Ok(())
    }

    async fn info(&self) -> Result<Self::Info, Self::Error> {
        Ok(MockVmInfo {
            state: "Running".to_string(),
        })
    }

    async fn pause(&self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn resume(&self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn counters(&self) -> Result<Self::Info, Self::Error> {
        Ok(MockVmInfo {
            state: "Running".to_string(),
        })
    }

    async fn ping(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

// ─── Mock process handle ──────────────────────────────────────────────────

pub struct MockProcess {
    tracker: MockCallTracker,
}

impl VmmProcess for MockProcess {
    async fn kill(&mut self) -> Result<(), VmError> {
        self.tracker.kills.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn cleanup(&mut self) -> Result<(), VmError> {
        self.tracker.cleanups.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

// ─── Mock backend factory ─────────────────────────────────────────────────

pub struct MockBackend {
    pub tracker: MockCallTracker,
    pub config: MockBackendConfig,
}

impl MockBackend {
    /// Create a new mock backend with default (all-success) config.
    pub fn new() -> (Self, MockCallTracker) {
        let tracker = MockCallTracker::new();
        let backend = Self {
            tracker: tracker.clone(),
            config: MockBackendConfig::default(),
        };
        (backend, tracker)
    }

    /// Create a mock backend with failure injection.
    pub fn with_config(config: MockBackendConfig) -> (Self, MockCallTracker) {
        let tracker = MockCallTracker::new();
        let backend = Self {
            tracker: tracker.clone(),
            config,
        };
        (backend, tracker)
    }
}

impl VmmBackend for MockBackend {
    type Client = MockVmm;
    type Process = MockProcess;

    async fn prepare(&self, _spec: &VmSpec) -> Result<(), VmError> {
        self.tracker.prepares.fetch_add(1, Ordering::Relaxed);
        if let Some(ref e) = self.config.prepare_error {
            return Err(VmError::Internal(e.clone()));
        }
        Ok(())
    }

    async fn spawn(
        &self,
        vm_id: &str,
    ) -> Result<(MockVmm, MockProcess, PathBuf), VmError> {
        self.tracker.spawns.fetch_add(1, Ordering::Relaxed);

        if let Some(ref e) = self.config.spawn_error {
            return Err(VmError::ProcessFailed(e.clone()));
        }

        let socket_path = PathBuf::from(format!("/tmp/mock/{vm_id}.sock"));
        let client = MockVmm {
            tracker: self.tracker.clone(),
            config: self.config.clone(),
        };
        let process = MockProcess {
            tracker: self.tracker.clone(),
        };

        Ok((client, process, socket_path))
    }

    fn build_config(&self, spec: &VmSpec) -> MockVmConfig {
        MockVmConfig {
            cpu: spec.cpu(),
            memory_mb: spec.memory_mb(),
            kernel_path: spec.kernel_path().to_string(),
            disk_image_path: spec.disk_image_path().to_string(),
        }
    }
}
