//! Explicit placement for pull-based assignment.
//!
//! v1 is explicit only: every desired VM is pinned to a target worker by the
//! publisher. [`explicit_map`] is a pure function from desired entries plus
//! declared inventory to per-worker slices — no bin-packing, no label
//! selectors, no resource fit. Unknown targets are rejected so a typo can
//! never silently drop a VM. Label/resource-aware scheduling is a later plan.

use std::{
    collections::HashMap,
    fmt::{Display, Formatter},
};

use super::{registry::WorkerId, store::DesiredEntry};

/// Explicit-mapping failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    /// An entry targets a worker outside the declared inventory.
    UnknownWorker(WorkerId),
}

impl Display for ScheduleError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownWorker(id) => {
                write!(f, "desired VM targets undeclared worker: {id}")
            }
        }
    }
}

impl std::error::Error for ScheduleError {}

/// Groups desired entries by explicit target worker, preserving payloads.
///
/// Entries are sorted by key within each worker slice so polls are
/// deterministic.
///
/// # Errors
///
/// Returns `ScheduleError::UnknownWorker` for the first entry whose target
/// is not in `known` (the declared inventory from
/// [`crate::registry::WorkerRegistry::known_workers`]).
pub fn explicit_map(
    entries: Vec<DesiredEntry>,
    known: &[WorkerId],
) -> Result<HashMap<WorkerId, Vec<DesiredEntry>>, ScheduleError> {
    let mut grouped: HashMap<WorkerId, Vec<DesiredEntry>> = HashMap::new();
    for entry in entries {
        if !known.contains(entry.target()) {
            return Err(ScheduleError::UnknownWorker(entry.target().clone()));
        }
        grouped
            .entry(entry.target().clone())
            .or_default()
            .push(entry);
    }
    for entries in grouped.values_mut() {
        entries.sort_by(|left, right| left.vm_key().as_str().cmp(right.vm_key().as_str()));
    }
    Ok(grouped)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{ScheduleError, explicit_map};
    use crate::{
        registry::WorkerId,
        store::{DesiredEntry, VmKey},
    };

    fn worker_id(name: &str) -> WorkerId {
        WorkerId::new(name).expect("test worker id must be valid")
    }

    fn entry(vm: &str, worker: &str) -> DesiredEntry {
        DesiredEntry::new(
            VmKey::new(vm).expect("test vm key must be valid"),
            worker_id(worker),
        )
    }

    #[test]
    fn groups_entries_by_explicit_target_in_sorted_order() {
        let grouped = explicit_map(
            vec![
                entry("vm-b", "w1").with_spec(vec![2]),
                entry("vm-a", "w1").with_spec(vec![1]),
                entry("vm-c", "w2"),
            ],
            &[worker_id("w1"), worker_id("w2")],
        )
        .expect("known targets must map");

        let mut expected: HashMap<WorkerId, Vec<DesiredEntry>> = HashMap::new();
        expected.insert(
            worker_id("w1"),
            vec![
                entry("vm-a", "w1").with_spec(vec![1]),
                entry("vm-b", "w1").with_spec(vec![2]),
            ],
        );
        expected.insert(worker_id("w2"), vec![entry("vm-c", "w2")]);
        assert_eq!(grouped, expected);
    }

    #[test]
    fn rejects_undeclared_targets() {
        let err = explicit_map(vec![entry("vm-a", "ghost")], &[worker_id("w1")])
            .expect_err("undeclared target must fail");
        assert_eq!(err, ScheduleError::UnknownWorker(worker_id("ghost")));
        assert!(
            format!("{err}").contains("ghost"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn maps_empty_entries_to_empty_grouping() {
        let grouped = explicit_map(Vec::new(), &[worker_id("w1")]).expect("empty input must map");
        assert!(grouped.is_empty());
    }
}
