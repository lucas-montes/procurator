//! Control-plane node: channel loop over a pure in-memory state machine.
//!
//! [`NodeState`] owns the declared registry, the assignment store, the
//! active generation metadata, and the last heartbeat report per worker. It
//! is directly unit-testable; [`Node`] only pumps channel messages through it
//! and always replies exactly once, so Cap'n Proto callers (T04) never hang.

use std::{collections::HashMap, net::SocketAddr};

use tokio::sync::mpsc::Receiver;

use crate::{
    dto::{
        ClusterSnapshot, NodeError, NodeEvent, NodeMessage, NodeReply, ObservedReport,
        WorkerStatusView,
    },
    registry::{
        DeclaredWorker, InMemoryWorkerRegistry, ObservedState, WorkerId, WorkerRegistry as _,
    },
    scheduler::explicit_map,
    store::{
        AssignmentStore as _, DesiredEntry, Generation, InMemoryAssignmentStore, VmKey,
        WorkerAssignment,
    },
};

/// Active desired-state metadata (commit plus intent it was evaluated from).
#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveState {
    generation: Generation,
    commit: String,
    intent_hash: String,
}

/// Pure in-memory control-plane state: publish/assignment/heartbeat/status.
#[derive(Debug)]
pub struct NodeState {
    registry: InMemoryWorkerRegistry,
    store: InMemoryAssignmentStore,
    active: Option<ActiveState>,
    reports: HashMap<WorkerId, ObservedReport>,
}

impl NodeState {
    /// Builds the state from pre-validated inventory
    /// (`Config::declared_workers`, T02). Duplicate ids cannot occur from
    /// that path; defensively, first declaration wins here.
    #[must_use]
    pub fn new(declared: Vec<DeclaredWorker>) -> Self {
        let mut registry = InMemoryWorkerRegistry::new();
        for worker in declared {
            let _ = registry.declare(worker);
        }
        Self {
            registry,
            store: InMemoryAssignmentStore::new(),
            active: None,
            reports: HashMap::new(),
        }
    }

    /// Validates and stores a new desired generation, replacing the full
    /// explicit assignment.
    ///
    /// # Errors
    ///
    /// Returns `NodeError::EmptyCommit` for blank commits,
    /// `NodeError::StaleGeneration` when `generation` is not newer than the
    /// active one, or `NodeError::Schedule` for undeclared targets. Nothing
    /// is stored on any error.
    pub fn publish(
        &mut self,
        commit: &str,
        generation: Generation,
        intent_hash: &str,
        entries: Vec<DesiredEntry>,
    ) -> Result<Generation, NodeError> {
        if commit.trim().is_empty() {
            return Err(NodeError::EmptyCommit);
        }
        if let Some(active) = &self.active
            && !generation.is_newer_than(active.generation.value())
        {
            return Err(NodeError::StaleGeneration {
                proposed: generation,
                active: active.generation,
            });
        }
        // Fail fast on undeclared targets before persisting anything.
        let known = self.registry.known_workers();
        let grouped = explicit_map(entries, &known).map_err(NodeError::Schedule)?;
        // Persist through the store so polls share its canonical grouping;
        // flattening is deterministic (sorted workers from the registry,
        // sorted entries from the scheduler) and preserves spec payloads.
        let mut ordered: Vec<DesiredEntry> = Vec::new();
        let mut targets: Vec<WorkerId> = grouped.keys().cloned().collect();
        targets.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        for target in targets {
            for entry in &grouped[&target] {
                ordered.push(entry.clone());
            }
        }
        self.store.publish(generation, ordered);
        self.active = Some(ActiveState {
            generation,
            commit: commit.to_string(),
            intent_hash: intent_hash.to_string(),
        });
        Ok(generation)
    }

    /// Returns the assignment slice for `worker` (empty, never an error,
    /// when unknown, unpublished, or up-to-date).
    #[must_use]
    pub fn assignment(&self, worker: &WorkerId, last_seen_generation: u64) -> WorkerAssignment {
        self.store.assignment_for(worker, last_seen_generation)
    }

    /// Merges one heartbeat report. Reports from undeclared workers are
    /// ignored: declaration is Nix-driven and deterministic.
    pub fn observe(&mut self, report: &ObservedReport) {
        if !self.registry.is_known(report.worker_id()) {
            return;
        }
        let running = u32::try_from(report.running().len()).unwrap_or(u32::MAX);
        self.registry.record_heartbeat(
            report.worker_id(),
            ObservedState::new(report.observed_generation(), report.is_healthy(), running),
        );
        self.reports
            .insert(report.worker_id().clone(), report.clone());
    }

    /// Returns one worker's status view, or `None` when undeclared.
    #[must_use]
    pub fn worker_snapshot(&self, worker: &WorkerId) -> Option<WorkerStatusView> {
        self.view_for(worker)
    }

    /// Returns the cluster-wide status snapshot in declared (sorted) order.
    #[must_use]
    pub fn cluster_snapshot(&self) -> ClusterSnapshot {
        let mut workers: Vec<WorkerStatusView> = Vec::new();
        let mut up_to_date: u32 = 0;
        for id in self.registry.known_workers() {
            if let Some(view) = self.view_for(&id) {
                if view.is_up_to_date() {
                    up_to_date = up_to_date.saturating_add(1);
                }
                workers.push(view);
            }
        }
        let total = u32::try_from(workers.len()).unwrap_or(u32::MAX);
        let (generation, commit, intent_hash) =
            self.active
                .as_ref()
                .map_or((None, String::new(), String::new()), |active| {
                    (
                        Some(active.generation),
                        active.commit.clone(),
                        active.intent_hash.clone(),
                    )
                });
        ClusterSnapshot::new(generation, commit, intent_hash, total, up_to_date, workers)
    }

    fn view_for(&self, worker: &WorkerId) -> Option<WorkerStatusView> {
        let snapshot = self.registry.snapshot(worker)?;
        // Desired slice as the worker itself would poll it. Generations
        // start at 1 (0 is never newer than anything in store poll
        // semantics), so `last_seen = 0` always yields the full slice.
        let desired: Vec<VmKey> = self.store.assignment_for(worker, 0).keys();
        let reported = self.reports.get(worker).cloned();
        let up_to_date = match (&self.active, &reported) {
            (Some(active), Some(report)) => {
                report.observed_generation() == active.generation.value()
                    && same_keys(report.running(), &desired)
            }
            (Some(_), None) | (None, _) => false,
        };
        Some(WorkerStatusView::new(
            worker.clone(),
            snapshot.declared().labels().to_vec(),
            snapshot.declared().addr_hint().map(str::to_string),
            desired,
            reported,
            up_to_date,
        ))
    }
}

/// Order-insensitive key-set equality for drift comparison.
fn same_keys(left: &[VmKey], right: &[VmKey]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut left: Vec<&str> = left.iter().map(VmKey::as_str).collect();
    let mut right: Vec<&str> = right.iter().map(VmKey::as_str).collect();
    left.sort_unstable();
    right.sort_unstable();
    left == right
}

/// Channel loop draining server requests into [`NodeState`].
pub struct Node {
    /// Channel to receive messages from the server
    receiver: Receiver<NodeMessage>,
    peers_addr: Vec<SocketAddr>,
    state: NodeState,
}

impl Node {
    /// Creates a node over `receiver`, pre-seeded with validated
    /// inventory (`Config::declared_workers`, T02).
    #[must_use]
    pub fn new(
        receiver: Receiver<NodeMessage>,
        peers_addr: Vec<SocketAddr>,
        declared: Vec<DeclaredWorker>,
    ) -> Self {
        Node {
            receiver,
            peers_addr,
            state: NodeState::new(declared),
        }
    }

    /// Main loop that processes messages from the server and sends command to the workers and orchestrates tasks
    pub async fn run(mut self) {
        tracing::info!(peers=?self.peers_addr, "Node started with peers");
        while let Some(message) = self.receiver.recv().await {
            let reply = self.handle(message.event());
            message.reply(reply);
        }
    }

    fn handle(&mut self, event: &NodeEvent) -> super::dto::NodeResult {
        match event {
            NodeEvent::PublishState {
                commit,
                generation,
                intent_hash,
                entries,
            } => self
                .state
                .publish(commit, *generation, intent_hash, entries.clone())
                .map(NodeReply::Published),
            NodeEvent::GetAssignment {
                worker_id,
                last_seen_generation,
            } => Ok(NodeReply::Assignment(
                self.state.assignment(worker_id, *last_seen_generation),
            )),
            NodeEvent::PushData { report } => {
                self.state.observe(report);
                Ok(NodeReply::Observed)
            }
            NodeEvent::GetClusterStatus => Ok(NodeReply::Cluster(self.state.cluster_snapshot())),
            NodeEvent::GetWorker { worker_id } => {
                Ok(NodeReply::Worker(self.state.worker_snapshot(worker_id)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::channel;

    use super::{Node, NodeState};
    use crate::{
        dto::{NodeError, NodeEvent, NodeReply, ObservedReport},
        registry::{DeclaredWorker, WorkerId},
        scheduler::ScheduleError,
        store::{DesiredEntry, Generation, VmKey},
    };

    fn worker_id(name: &str) -> WorkerId {
        WorkerId::new(name).expect("test worker id must be valid")
    }

    fn key(name: &str) -> VmKey {
        VmKey::new(name).expect("test vm key must be valid")
    }

    fn entry(vm: &str, worker: &str) -> DesiredEntry {
        DesiredEntry::new(key(vm), worker_id(worker))
    }

    fn declared(name: &str) -> DeclaredWorker {
        DeclaredWorker::new(worker_id(name))
    }

    fn state() -> NodeState {
        NodeState::new(vec![declared("w1"), declared("w2")])
    }

    fn publish(state: &mut NodeState) {
        state
            .publish(
                "abc",
                Generation::new(1),
                "intent-1",
                vec![entry("vm-a", "w1")],
            )
            .expect("test publish must succeed");
    }

    #[test]
    fn publish_assignment_roundtrip() {
        let mut state = state();
        publish(&mut state);

        let w1 = state.assignment(&worker_id("w1"), 0);
        assert_eq!(w1.generation(), Some(Generation::new(1)));
        assert_eq!(w1.keys(), vec![key("vm-a")]);

        let w2 = state.assignment(&worker_id("w2"), 0);
        assert_eq!(w2.generation(), Some(Generation::new(1)));
        assert!(w2.is_empty());

        let ghost = state.assignment(&worker_id("ghost"), 0);
        assert_eq!(ghost.generation(), Some(Generation::new(1)));
        assert!(ghost.is_empty());
    }

    #[test]
    fn rejects_stale_generations_and_empty_commits() {
        let mut state = state();
        publish(&mut state);

        let err = state
            .publish(
                "def",
                Generation::new(1),
                "intent-2",
                vec![entry("vm-b", "w1")],
            )
            .expect_err("same generation must be stale");
        assert_eq!(
            err,
            NodeError::StaleGeneration {
                proposed: Generation::new(1),
                active: Generation::new(1),
            }
        );

        let err = state
            .publish(
                "   ",
                Generation::new(2),
                "intent-2",
                vec![entry("vm-b", "w1")],
            )
            .expect_err("blank commit must fail");
        assert_eq!(err, NodeError::EmptyCommit);

        // Failed publishes store nothing: w1 still sees only vm-a at g1.
        let w1 = state.assignment(&worker_id("w1"), 0);
        assert_eq!(w1.generation(), Some(Generation::new(1)));
        assert_eq!(w1.keys(), vec![key("vm-a")]);
    }

    #[test]
    fn rejects_undeclared_targets_without_storing() {
        let mut state = state();
        let err = state
            .publish(
                "abc",
                Generation::new(1),
                "intent-1",
                vec![entry("vm-a", "ghost")],
            )
            .expect_err("undeclared target must fail");
        assert_eq!(
            err,
            NodeError::Schedule(ScheduleError::UnknownWorker(worker_id("ghost")))
        );

        let snapshot = state.cluster_snapshot();
        assert_eq!(snapshot.generation(), None);
        assert!(!snapshot.is_converged() || snapshot.total_workers() == 2);
    }

    #[test]
    fn heartbeat_merge_and_convergence() {
        let mut state = state();
        publish(&mut state);

        state.observe(&ObservedReport::new(
            worker_id("w1"),
            1,
            vec![key("vm-a")],
            true,
        ));
        // Heartbeats from undeclared workers are ignored.
        state.observe(&ObservedReport::new(
            worker_id("ghost"),
            1,
            vec![key("vm-x")],
            true,
        ));

        let snapshot = state.cluster_snapshot();
        assert_eq!(snapshot.generation(), Some(Generation::new(1)));
        assert_eq!(snapshot.commit(), "abc");
        assert_eq!(snapshot.intent_hash(), "intent-1");
        assert_eq!(snapshot.total_workers(), 2);
        assert_eq!(snapshot.up_to_date_workers(), 1);
        assert_eq!(snapshot.convergence_percent(), 50);
        assert!(!snapshot.is_converged());

        let w1 = state
            .worker_snapshot(&worker_id("w1"))
            .expect("declared worker must have a view");
        assert!(w1.is_up_to_date());
        assert!(w1.reported().expect("report merged").is_healthy());
        assert_eq!(w1.desired(), &[key("vm-a")]);

        let w2 = state
            .worker_snapshot(&worker_id("w2"))
            .expect("declared worker must have a view");
        assert!(!w2.is_up_to_date());

        assert!(state.worker_snapshot(&worker_id("ghost")).is_none());
    }

    #[test]
    fn empty_inventory_converges_vacuously() {
        let state = NodeState::new(Vec::new());
        let snapshot = state.cluster_snapshot();
        assert_eq!(snapshot.total_workers(), 0);
        assert_eq!(snapshot.convergence_percent(), 100);
        assert!(snapshot.is_converged());
    }

    #[tokio::test]
    async fn loop_replies_to_every_event() {
        use crate::dto::NodeMessenger;

        let (tx, rx) = channel(8);
        let node = Node::new(rx, Vec::new(), vec![declared("w1"), declared("w2")]);
        let task = tokio::spawn(node.run());
        let messenger = NodeMessenger::from(tx);

        let reply = messenger
            .request(NodeEvent::PublishState {
                commit: "abc".to_string(),
                generation: Generation::new(1),
                intent_hash: "intent-1".to_string(),
                entries: vec![entry("vm-a", "w1")],
            })
            .await
            .expect("publish request must succeed");
        assert_eq!(reply, Ok(NodeReply::Published(Generation::new(1))));

        let reply = messenger
            .request(NodeEvent::GetAssignment {
                worker_id: worker_id("w1"),
                last_seen_generation: 0,
            })
            .await
            .expect("assignment request must succeed");
        match reply {
            Ok(NodeReply::Assignment(assignment)) => {
                assert_eq!(assignment.keys(), vec![key("vm-a")]);
            }
            other => panic!("unexpected assignment reply: {other:?}"),
        }

        let reply = messenger
            .request(NodeEvent::PushData {
                report: ObservedReport::new(worker_id("w1"), 1, vec![key("vm-a")], true),
            })
            .await
            .expect("push request must succeed");
        assert_eq!(reply, Ok(NodeReply::Observed));

        let reply = messenger
            .request(NodeEvent::GetClusterStatus)
            .await
            .expect("status request must succeed");
        match reply {
            Ok(NodeReply::Cluster(snapshot)) => {
                assert_eq!(snapshot.convergence_percent(), 50);
            }
            other => panic!("unexpected cluster reply: {other:?}"),
        }

        let reply = messenger
            .request(NodeEvent::GetWorker {
                worker_id: worker_id("ghost"),
            })
            .await
            .expect("worker request must succeed");
        assert_eq!(reply, Ok(NodeReply::Worker(None)));

        drop(messenger);
        task.await.expect("node task must exit when senders drop");
    }
}
