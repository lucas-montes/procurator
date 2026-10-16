//! Worker registry boundary for pull-based assignment.
//!
//! The registry combines Nix-declared worker inventory (deterministic, see T02)
//! with dynamic liveness observed from `pushData` heartbeats (see T03).
//!
//! Transport note: this boundary is direction-agnostic. Today's transport is
//! pull (workers poll `getAssignment`, report `pushData`). A future `PushRpc`
//! transport can reuse the same declared inventory and observed snapshots and
//! only replace how assignments travel; see `crate::store::AssignmentSource`.

use std::collections::HashMap;
use std::fmt::{Display, Formatter};

/// Strongly-typed worker identity, matching the Nix-declared worker name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkerId(String);

impl WorkerId {
    /// Creates a worker id from a raw name.
    ///
    /// # Errors
    ///
    /// Returns `RegistryError::EmptyId` when `id` is empty or whitespace-only.
    pub fn new(id: impl Into<String>) -> Result<Self, RegistryError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(RegistryError::EmptyId);
        }
        Ok(Self(id))
    }

    /// Borrows the raw worker name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for WorkerId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A worker declared in Nix configuration (deterministic inventory, see T02).
///
/// `addr_hint` is informational only: pull needs no dialable address, but the
/// Nix `cluster.vms` deployment address is kept for logs, status views, and a
/// future push transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredWorker {
    id: WorkerId,
    labels: Vec<String>,
    addr_hint: Option<String>,
}

impl DeclaredWorker {
    /// Declares a worker with no labels and no address hint.
    #[must_use]
    pub fn new(id: WorkerId) -> Self {
        Self {
            id,
            labels: Vec::new(),
            addr_hint: None,
        }
    }

    /// Attaches scheduling labels (reserved for a later scheduler plan).
    #[must_use]
    pub fn with_labels(mut self, labels: Vec<String>) -> Self {
        self.labels = labels;
        self
    }

    /// Attaches the informational deployment address hint from Nix.
    #[must_use]
    pub fn with_addr_hint(mut self, addr: impl Into<String>) -> Self {
        self.addr_hint = Some(addr.into());
        self
    }

    /// Borrows the worker id.
    #[must_use]
    pub fn id(&self) -> &WorkerId {
        &self.id
    }

    /// Borrows the scheduling labels.
    #[must_use]
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// Borrows the informational address hint, if any.
    #[must_use]
    pub fn addr_hint(&self) -> Option<&str> {
        self.addr_hint.as_deref()
    }
}

/// Dynamic liveness last reported by a worker heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedState {
    last_seen_generation: u64,
    healthy: bool,
    running_vms: u32,
}

impl ObservedState {
    /// Records one heartbeat observation.
    #[must_use]
    pub fn new(last_seen_generation: u64, healthy: bool, running_vms: u32) -> Self {
        Self {
            last_seen_generation,
            healthy,
            running_vms,
        }
    }

    /// Returns the generation the worker had applied at heartbeat time.
    #[must_use]
    pub fn last_seen_generation(&self) -> u64 {
        self.last_seen_generation
    }

    /// Returns whether the worker reported itself healthy.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.healthy
    }

    /// Returns the worker-reported running VM count.
    #[must_use]
    pub fn running_vms(&self) -> u32 {
        self.running_vms
    }
}

/// Combined declared + observed view of one worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerSnapshot {
    declared: DeclaredWorker,
    observed: Option<ObservedState>,
}

impl WorkerSnapshot {
    /// Borrows the declared worker.
    #[must_use]
    pub fn declared(&self) -> &DeclaredWorker {
        &self.declared
    }

    /// Borrows the last observed state, if the worker ever sent a heartbeat.
    #[must_use]
    pub fn observed(&self) -> Option<&ObservedState> {
        self.observed.as_ref()
    }
}

/// Registry failures for declaration-time validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// Worker id was empty or whitespace-only.
    EmptyId,
    /// A worker with the same id is already declared.
    Duplicate(WorkerId),
}

impl Display for RegistryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyId => f.write_str("worker id must not be empty"),
            Self::Duplicate(id) => write!(f, "worker id is already declared: {id}"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Declared inventory plus heartbeat liveness for pull-based assignment.
///
/// Implementations must be deterministic: [`WorkerRegistry::known_workers`]
/// returns ids in sorted order so explicit-map scheduling (T03) is stable.
pub trait WorkerRegistry {
    /// Declares a worker from Nix inventory.
    ///
    /// # Errors
    ///
    /// Returns `RegistryError::Duplicate` when the id is already declared.
    fn declare(&mut self, worker: DeclaredWorker) -> Result<(), RegistryError>;

    /// Merges one heartbeat observation. Heartbeats from undeclared workers
    /// are ignored: declaration is Nix-driven and deterministic.
    fn record_heartbeat(&mut self, id: &WorkerId, observed: ObservedState);

    /// Returns the combined declared + observed view, if the worker is declared.
    fn snapshot(&self, id: &WorkerId) -> Option<WorkerSnapshot>;

    /// Returns all declared worker ids in sorted order.
    fn known_workers(&self) -> Vec<WorkerId>;

    /// Returns whether the id is declared.
    fn is_known(&self, id: &WorkerId) -> bool;
}

/// In-memory [`WorkerRegistry`] owned by `Node` at runtime (see T03).
///
/// This is the fake implementation proving the trait shape for T01.
#[derive(Debug, Default)]
pub struct InMemoryWorkerRegistry {
    entries: HashMap<WorkerId, (DeclaredWorker, Option<ObservedState>)>,
}

impl InMemoryWorkerRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl WorkerRegistry for InMemoryWorkerRegistry {
    fn declare(&mut self, worker: DeclaredWorker) -> Result<(), RegistryError> {
        if self.entries.contains_key(worker.id()) {
            return Err(RegistryError::Duplicate(worker.id().clone()));
        }
        self.entries.insert(worker.id().clone(), (worker, None));
        Ok(())
    }

    fn record_heartbeat(&mut self, id: &WorkerId, observed: ObservedState) {
        if let Some((_, slot)) = self.entries.get_mut(id) {
            *slot = Some(observed);
        }
    }

    fn snapshot(&self, id: &WorkerId) -> Option<WorkerSnapshot> {
        self.entries
            .get(id)
            .map(|(declared, observed)| WorkerSnapshot {
                declared: declared.clone(),
                observed: *observed,
            })
    }

    fn known_workers(&self) -> Vec<WorkerId> {
        let mut ids: Vec<WorkerId> = self.entries.keys().cloned().collect();
        ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        ids
    }

    fn is_known(&self, id: &WorkerId) -> bool {
        self.entries.contains_key(id)
    }
}

#[cfg(test)]
mod tests {
    use super::{DeclaredWorker, InMemoryWorkerRegistry, ObservedState, RegistryError, WorkerId};

    use super::WorkerRegistry as _;

    fn worker_id(name: &str) -> WorkerId {
        WorkerId::new(name).expect("test worker id must be valid")
    }

    #[test]
    fn rejects_empty_worker_id() {
        assert_eq!(WorkerId::new(""), Err(RegistryError::EmptyId));
        assert_eq!(WorkerId::new("   "), Err(RegistryError::EmptyId));
    }

    #[test]
    fn rejects_duplicate_declaration() {
        let mut registry = InMemoryWorkerRegistry::new();
        registry
            .declare(DeclaredWorker::new(worker_id("w1")))
            .expect("first declaration must succeed");

        let err = registry
            .declare(DeclaredWorker::new(worker_id("w1")))
            .expect_err("duplicate declaration must fail");
        assert_eq!(err, RegistryError::Duplicate(worker_id("w1")));
    }

    #[test]
    fn lists_known_workers_in_sorted_order() {
        let mut registry = InMemoryWorkerRegistry::new();
        for name in ["w2", "w1", "w10"] {
            registry
                .declare(DeclaredWorker::new(worker_id(name)))
                .expect("declaration must succeed");
        }

        let ids: Vec<String> = registry
            .known_workers()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(ids, vec!["w1", "w10", "w2"]);
        assert!(registry.is_known(&worker_id("w1")));
        assert!(!registry.is_known(&worker_id("ghost")));
    }

    #[test]
    fn merges_heartbeats_for_declared_workers_only() {
        let mut registry = InMemoryWorkerRegistry::new();
        registry
            .declare(
                DeclaredWorker::new(worker_id("w1"))
                    .with_addr_hint("192.168.1.11:8080")
                    .with_labels(vec!["gpu".to_string()]),
            )
            .expect("declaration must succeed");

        let observed = ObservedState::new(7, true, 2);
        registry.record_heartbeat(&worker_id("w1"), observed);
        // Heartbeats from undeclared workers are ignored (Nix-driven inventory).
        registry.record_heartbeat(&worker_id("ghost"), observed);

        let snapshot = registry
            .snapshot(&worker_id("w1"))
            .expect("declared worker must have a snapshot");
        assert_eq!(snapshot.observed(), Some(&observed));
        assert_eq!(snapshot.declared().addr_hint(), Some("192.168.1.11:8080"));
        assert!(registry.snapshot(&worker_id("ghost")).is_none());
    }
}
