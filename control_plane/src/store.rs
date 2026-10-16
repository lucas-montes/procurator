//! Assignment store boundary for pull-based task delivery.
//!
//! Explicit-target contract (v1, plan T09):
//! - Every desired VM entry carries a stable [`VmKey`] (logical identity),
//!   an explicit target [`WorkerId`], and opaque spec bytes retained for
//!   `getAssignment` rebuilds.
//! - [`AssignmentStore::assignment_for`] filters by target worker. Unknown
//!   workers and up-to-date workers receive an empty slice — never an error —
//!   so polling stays idempotent and cheap.
//! - Wire bridge: Cap'n Proto `VmSpec` payloads map onto [`VmKey`] via the
//!   additive `id` field, targets via `targetWorker`, and bytes roundtrip
//!   opaquely (see `server.rs`, plan T09).
//!
//! Transport note: [`AssignmentSource`] is the direction-agnostic read path.
//! Pull serves it from the poll handler; a future `PushRpc` transport
//! implements the same trait by sending instead of waiting for polls.
//!
//! [`WorkerId`]: super::registry::WorkerId

use std::collections::HashMap;
use std::fmt::{Display, Formatter};

use super::registry::WorkerId;

/// Monotonic desired-state generation published by the control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Generation(u64);

impl Generation {
    /// Wraps a raw generation number.
    #[must_use]
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw generation number.
    #[must_use]
    pub fn value(self) -> u64 {
        self.0
    }

    /// Returns whether `self` is newer than the given generation.
    #[must_use]
    pub fn is_newer_than(self, other: u64) -> bool {
        self.0 > other
    }
}

impl Display for Generation {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Stable logical identity of one desired VM.
///
/// The wire-to-key mapping is defined in T03 (see module docs); the store
/// only orders and filters by key, never interpreting the VM payload itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VmKey(String);

impl VmKey {
    /// Creates a VM key from a raw identity string.
    ///
    /// # Errors
    ///
    /// Returns `StoreError::EmptyVmKey` when `key` is empty or whitespace-only.
    pub fn new(key: impl Into<String>) -> Result<Self, StoreError> {
        let key = key.into();
        if key.trim().is_empty() {
            return Err(StoreError::EmptyVmKey);
        }
        Ok(Self(key))
    }

    /// Borrows the raw key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for VmKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One desired VM pinned to an explicit target worker, carrying its opaque
/// spec bytes for `getAssignment` rebuilds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredEntry {
    vm_key: VmKey,
    target: WorkerId,
    spec: Vec<u8>,
}

impl DesiredEntry {
    /// Pins `vm_key` to `target` for explicit placement (no spec bytes yet).
    #[must_use]
    pub fn new(vm_key: VmKey, target: WorkerId) -> Self {
        Self {
            vm_key,
            target,
            spec: Vec::new(),
        }
    }

    /// Attaches the opaque serialized `VmSpec` bytes.
    #[must_use]
    pub fn with_spec(mut self, spec: Vec<u8>) -> Self {
        self.spec = spec;
        self
    }

    /// Borrows the VM key.
    #[must_use]
    pub fn vm_key(&self) -> &VmKey {
        &self.vm_key
    }

    /// Borrows the explicit target worker.
    #[must_use]
    pub fn target(&self) -> &WorkerId {
        &self.target
    }
}

/// One assigned VM: stable key plus opaque spec bytes for the wire rebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignedVm {
    key: VmKey,
    spec: Vec<u8>,
}

impl AssignedVm {
    /// Builds an assigned VM from its key and opaque spec bytes.
    #[must_use]
    // Covered by unit tests; reserved for future producers.
    #[allow(dead_code)]
    pub fn new(key: VmKey, spec: Vec<u8>) -> Self {
        Self { key, spec }
    }

    /// Borrows the VM key.
    #[must_use]
    pub fn key(&self) -> &VmKey {
        &self.key
    }

    /// Borrows the opaque spec bytes.
    #[must_use]
    pub fn spec(&self) -> &[u8] {
        &self.spec
    }
}

/// Per-worker assignment slice handed to one poll response.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkerAssignment {
    generation: Option<Generation>,
    vms: Vec<AssignedVm>,
}

impl WorkerAssignment {
    /// Builds an assignment slice for `generation` (or `None` when nothing
    /// was ever published).
    #[must_use]
    pub fn new(generation: Option<Generation>, vms: Vec<AssignedVm>) -> Self {
        Self { generation, vms }
    }

    /// Returns the generation this slice belongs to, if any was published.
    #[must_use]
    pub fn generation(&self) -> Option<Generation> {
        self.generation
    }

    /// Borrows the assigned VMs in stable key order.
    #[must_use]
    pub fn vms(&self) -> &[AssignedVm] {
        &self.vms
    }

    /// Returns the desired VM keys in stable order.
    #[must_use]
    pub fn keys(&self) -> Vec<VmKey> {
        self.vms.iter().map(|vm| vm.key.clone()).collect()
    }

    /// Returns whether the slice carries no desired VMs.
    #[must_use]
    // Covered by unit tests; reserved for future consumers.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.vms.is_empty()
    }
}

/// Store failures for key validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// VM key was empty or whitespace-only.
    EmptyVmKey,
}

impl Display for StoreError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyVmKey => f.write_str("vm key must not be empty"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Stored desired state with explicit per-worker routing.
///
/// `publish` replaces the full desired state for a generation: the store is
/// a ledger of explicit intent, and monotonicity validation belongs to `Node`
/// (T03), not to this boundary.
pub trait AssignmentStore {
    /// Replaces the full desired state for `generation`.
    fn publish(&mut self, generation: Generation, entries: Vec<DesiredEntry>);

    /// Returns the active generation, if anything was published.
    // Covered by unit tests; push transports read through this boundary.
    #[allow(dead_code)]
    fn active_generation(&self) -> Option<Generation>;

    /// Returns the assignment slice for `worker` given its last applied
    /// generation. Empty — never an error — when nothing was published, the
    /// worker is unknown, or the worker is already up-to-date. The returned
    /// generation echoes the active one whenever known.
    fn assignment_for(&self, worker: &WorkerId, last_seen_generation: u64) -> WorkerAssignment;
}

/// Direction-agnostic assignment read path (pull now, push later).
///
/// Every [`AssignmentStore`] is an [`AssignmentSource`] via the blanket impl
/// below; a future `PushRpc` transport implements this trait directly without
/// owning a store.
// Transport seam with no pull-mode users by design; a future push transport
// implements this trait directly without owning a store.
#[allow(dead_code)]
pub trait AssignmentSource {
    /// Returns the assignment slice for `worker` (see
    /// [`AssignmentStore::assignment_for`]).
    fn assignment_for(&self, worker: &WorkerId, last_seen_generation: u64) -> WorkerAssignment;

    /// Returns the active generation (see
    /// [`AssignmentStore::active_generation`]).
    fn active_generation(&self) -> Option<Generation>;
}

impl<T: AssignmentStore> AssignmentSource for T {
    fn assignment_for(&self, worker: &WorkerId, last_seen_generation: u64) -> WorkerAssignment {
        AssignmentStore::assignment_for(self, worker, last_seen_generation)
    }

    fn active_generation(&self) -> Option<Generation> {
        AssignmentStore::active_generation(self)
    }
}

/// In-memory [`AssignmentStore`] owned by `Node` at runtime (see T03).
///
/// This is the fake implementation proving the trait shape for T01.
#[derive(Debug, Default)]
pub struct InMemoryAssignmentStore {
    active: Option<Generation>,
    per_worker: HashMap<WorkerId, Vec<AssignedVm>>,
}

impl InMemoryAssignmentStore {
    /// Creates an empty store with no active generation.
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: None,
            per_worker: HashMap::new(),
        }
    }
}

impl AssignmentStore for InMemoryAssignmentStore {
    fn publish(&mut self, generation: Generation, entries: Vec<DesiredEntry>) {
        let mut per_worker: HashMap<WorkerId, Vec<AssignedVm>> = HashMap::new();
        for entry in entries {
            per_worker
                .entry(entry.target)
                .or_default()
                .push(AssignedVm {
                    key: entry.vm_key,
                    spec: entry.spec,
                });
        }
        // Keep each worker slice in stable key order for deterministic polls.
        for vms in per_worker.values_mut() {
            vms.sort_by(|left, right| left.key.as_str().cmp(right.key.as_str()));
        }
        self.per_worker = per_worker;
        self.active = Some(generation);
    }

    fn active_generation(&self) -> Option<Generation> {
        self.active
    }

    fn assignment_for(&self, worker: &WorkerId, last_seen_generation: u64) -> WorkerAssignment {
        match self.active {
            None => WorkerAssignment::new(None, Vec::new()),
            Some(active) if !active.is_newer_than(last_seen_generation) => {
                WorkerAssignment::new(Some(active), Vec::new())
            }
            Some(active) => WorkerAssignment::new(
                Some(active),
                self.per_worker.get(worker).cloned().unwrap_or_default(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AssignmentStore as _, DesiredEntry, Generation, InMemoryAssignmentStore, StoreError, VmKey,
        WorkerAssignment,
    };
    use crate::registry::WorkerId;

    fn worker_id(name: &str) -> WorkerId {
        WorkerId::new(name).expect("test worker id must be valid")
    }

    fn vm_key(name: &str) -> VmKey {
        VmKey::new(name).expect("test vm key must be valid")
    }

    fn entry(vm: &str, worker: &str) -> DesiredEntry {
        DesiredEntry::new(vm_key(vm), worker_id(worker))
    }

    #[test]
    fn rejects_empty_vm_key() {
        assert_eq!(VmKey::new(""), Err(StoreError::EmptyVmKey));
        assert_eq!(VmKey::new("  "), Err(StoreError::EmptyVmKey));
    }

    #[test]
    fn returns_empty_assignment_before_any_publish() {
        let store = InMemoryAssignmentStore::new();
        assert_eq!(store.active_generation(), None);

        let assignment = store.assignment_for(&worker_id("w1"), 0);
        assert_eq!(assignment, WorkerAssignment::new(None, Vec::new()));
        assert!(assignment.is_empty());
    }

    #[test]
    fn filters_assignments_by_explicit_target() {
        let mut store = InMemoryAssignmentStore::new();
        store.publish(
            Generation::new(1),
            vec![
                entry("vm-a", "w1"),
                entry("vm-b", "w1"),
                entry("vm-c", "w2"),
            ],
        );

        assert_eq!(store.active_generation(), Some(Generation::new(1)));

        let w1 = store.assignment_for(&worker_id("w1"), 0);
        assert_eq!(w1.generation(), Some(Generation::new(1)));
        assert_eq!(w1.keys(), vec![vm_key("vm-a"), vm_key("vm-b")]);

        let w2 = store.assignment_for(&worker_id("w2"), 0);
        assert_eq!(w2.keys(), vec![vm_key("vm-c")]);

        // Unknown workers get an empty slice with the active generation,
        // never an error.
        let ghost = store.assignment_for(&worker_id("ghost"), 0);
        assert_eq!(
            ghost,
            WorkerAssignment::new(Some(Generation::new(1)), Vec::new())
        );
    }

    #[test]
    fn up_to_date_workers_get_an_empty_slice() {
        let mut store = InMemoryAssignmentStore::new();
        store.publish(Generation::new(3), vec![entry("vm-a", "w1")]);

        let current = store.assignment_for(&worker_id("w1"), 3);
        assert_eq!(
            current,
            WorkerAssignment::new(Some(Generation::new(3)), Vec::new())
        );

        let stale = store.assignment_for(&worker_id("w1"), 2);
        assert_eq!(stale.keys(), vec![vm_key("vm-a")]);
    }

    #[test]
    fn republish_replaces_the_full_desired_state() {
        let mut store = InMemoryAssignmentStore::new();
        store.publish(
            Generation::new(1),
            vec![entry("vm-a", "w1"), entry("vm-b", "w1")],
        );
        store.publish(Generation::new(2), vec![entry("vm-b", "w1")]);

        let w1 = store.assignment_for(&worker_id("w1"), 1);
        assert_eq!(w1.generation(), Some(Generation::new(2)));
        assert_eq!(w1.keys(), vec![vm_key("vm-b")]);
    }

    #[test]
    fn retains_spec_payloads_per_worker() {
        use super::AssignedVm;

        let mut store = InMemoryAssignmentStore::new();
        store.publish(
            Generation::new(1),
            vec![
                entry("vm-a", "w1").with_spec(vec![1, 2, 3]),
                entry("vm-b", "w1").with_spec(vec![4]),
            ],
        );

        let w1 = store.assignment_for(&worker_id("w1"), 0);
        assert_eq!(
            w1.vms(),
            &[
                AssignedVm::new(vm_key("vm-a"), vec![1, 2, 3]),
                AssignedVm::new(vm_key("vm-b"), vec![4]),
            ]
        );

        // Republishing without an entry prunes its payload with it.
        store.publish(Generation::new(2), vec![entry("vm-b", "w1")]);
        let w1 = store.assignment_for(&worker_id("w1"), 1);
        assert_eq!(w1.vms(), &[AssignedVm::new(vm_key("vm-b"), vec![])]);
    }
}
