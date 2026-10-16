//! Channel messages between the Cap'n Proto server and the control-plane node.
//!
//! Events carry validated domain types (`WorkerId`, `DesiredEntry`,
//! `ObservedReport`); wire parsing and its failures belong to `server.rs`
//! (T04). Replies are typed per event via [`NodeReply`]; every event the loop
//! receives is answered exactly once.

use std::fmt::{Display, Formatter};

use tokio::sync::{
    mpsc::Sender,
    oneshot::{self, Receiver},
};

use super::{
    registry::WorkerId,
    scheduler::ScheduleError,
    store::{DesiredEntry, Generation, VmKey, WorkerAssignment},
};

/// Heartbeat report from one worker, already mapped from the wire `pushData`
/// payload by the server (T04).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedReport {
    worker_id: WorkerId,
    observed_generation: u64,
    running: Vec<VmKey>,
    healthy: bool,
}

impl ObservedReport {
    /// Builds a heartbeat report for `worker_id`.
    #[must_use]
    pub fn new(
        worker_id: WorkerId,
        observed_generation: u64,
        running: Vec<VmKey>,
        healthy: bool,
    ) -> Self {
        Self {
            worker_id,
            observed_generation,
            running,
            healthy,
        }
    }

    /// Borrows the reporting worker id.
    #[must_use]
    pub fn worker_id(&self) -> &WorkerId {
        &self.worker_id
    }

    /// Returns the generation the worker had applied at heartbeat time.
    #[must_use]
    pub fn observed_generation(&self) -> u64 {
        self.observed_generation
    }

    /// Borrows the worker-reported running VM keys.
    #[must_use]
    pub fn running(&self) -> &[VmKey] {
        &self.running
    }

    /// Returns whether the worker reported itself healthy.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.healthy
    }
}

/// Per-worker status view for cluster snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerStatusView {
    worker_id: WorkerId,
    labels: Vec<String>,
    addr_hint: Option<String>,
    desired: Vec<VmKey>,
    reported: Option<ObservedReport>,
    up_to_date: bool,
}

impl WorkerStatusView {
    /// Builds a per-worker status view.
    #[must_use]
    pub fn new(
        worker_id: WorkerId,
        labels: Vec<String>,
        addr_hint: Option<String>,
        desired: Vec<VmKey>,
        reported: Option<ObservedReport>,
        up_to_date: bool,
    ) -> Self {
        Self {
            worker_id,
            labels,
            addr_hint,
            desired,
            reported,
            up_to_date,
        }
    }

    /// Borrows the worker id.
    #[must_use]
    pub fn worker_id(&self) -> &WorkerId {
        &self.worker_id
    }

    /// Borrows the declared scheduling labels.
    #[must_use]
    // Pull-domain read API: covered by unit tests; reserved for CLI views.
    #[allow(dead_code)]
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// Borrows the informational address hint, if any.
    #[must_use]
    // Pull-domain read API: covered by unit tests; reserved for CLI views.
    #[allow(dead_code)]
    pub fn addr_hint(&self) -> Option<&str> {
        self.addr_hint.as_deref()
    }

    /// Borrows the desired VM keys for the active generation.
    #[must_use]
    // Pull-domain read API: covered by unit tests; reserved for CLI views.
    #[allow(dead_code)]
    pub fn desired(&self) -> &[VmKey] {
        &self.desired
    }

    /// Borrows the last heartbeat report, if any.
    #[must_use]
    pub fn reported(&self) -> Option<&ObservedReport> {
        self.reported.as_ref()
    }

    /// Returns whether the worker is fully converged on the active
    /// generation (observed generation matches and reported running VMs
    /// equal the desired set, order-insensitively).
    #[must_use]
    pub fn is_up_to_date(&self) -> bool {
        self.up_to_date
    }
}

/// Cluster-wide status snapshot served to read-only clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterSnapshot {
    generation: Option<Generation>,
    commit: String,
    intent_hash: String,
    total_workers: u32,
    up_to_date_workers: u32,
    workers: Vec<WorkerStatusView>,
}

impl ClusterSnapshot {
    /// Builds a cluster snapshot.
    #[must_use]
    pub fn new(
        generation: Option<Generation>,
        commit: String,
        intent_hash: String,
        total_workers: u32,
        up_to_date_workers: u32,
        workers: Vec<WorkerStatusView>,
    ) -> Self {
        Self {
            generation,
            commit,
            intent_hash,
            total_workers,
            up_to_date_workers,
            workers,
        }
    }

    /// Returns the active generation, if anything was published.
    #[must_use]
    pub fn generation(&self) -> Option<Generation> {
        self.generation
    }

    /// Borrows the active commit hash (empty when nothing published).
    #[must_use]
    pub fn commit(&self) -> &str {
        &self.commit
    }

    /// Borrows the active intent hash (empty when nothing published).
    #[must_use]
    // Pull-domain read API: covered by unit tests; reserved for CLI views.
    #[allow(dead_code)]
    pub fn intent_hash(&self) -> &str {
        &self.intent_hash
    }

    /// Returns the number of declared workers.
    #[must_use]
    // Pull-domain read API: covered by unit tests; reserved for CLI views.
    #[allow(dead_code)]
    pub fn total_workers(&self) -> u32 {
        self.total_workers
    }

    /// Returns the number of workers converged on the active generation.
    #[must_use]
    // Pull-domain read API: covered by unit tests; reserved for CLI views.
    #[allow(dead_code)]
    pub fn up_to_date_workers(&self) -> u32 {
        self.up_to_date_workers
    }

    /// Borrows the per-worker views in declared (sorted) order.
    #[must_use]
    pub fn workers(&self) -> &[WorkerStatusView] {
        &self.workers
    }

    /// Returns the convergence percentage over declared workers: 0–100.
    /// With no declared workers there is nothing to converge, so 100.
    #[must_use]
    pub fn convergence_percent(&self) -> u32 {
        if self.total_workers == 0 {
            return 100;
        }
        (self.up_to_date_workers * 100) / self.total_workers
    }

    /// Returns whether every declared worker is converged (vacuously true
    /// with no declared workers, matching [`Self::convergence_percent`]).
    #[must_use]
    // Pull-domain read API: covered by unit tests; reserved for CLI views.
    #[allow(dead_code)]
    pub fn is_converged(&self) -> bool {
        self.up_to_date_workers == self.total_workers
    }
}

/// Events the RPC server forwards to the control-plane node.
#[derive(Debug)]
pub enum NodeEvent {
    /// A new desired generation was published (explicit-target entries).
    PublishState {
        /// Commit hash the generation was evaluated from.
        commit: String,
        /// Monotonic generation number proposed by the publisher.
        generation: Generation,
        /// Content hash of the evaluated intent.
        intent_hash: String,
        /// Desired VMs pinned to explicit target workers.
        entries: Vec<DesiredEntry>,
    },
    /// A worker polls for its assignment slice.
    GetAssignment {
        /// Polling worker id.
        worker_id: WorkerId,
        /// Last generation the worker applied.
        last_seen_generation: u64,
    },
    /// A worker reports observed state.
    PushData {
        /// Mapped heartbeat report.
        report: ObservedReport,
    },
    /// A CLI client requests the cluster snapshot.
    GetClusterStatus,
    /// A CLI client requests one worker's snapshot.
    GetWorker {
        /// Requested worker id.
        worker_id: WorkerId,
    },
}

/// Typed replies, one per event variant in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeReply {
    /// New active generation after a publish.
    Published(Generation),
    /// Assignment slice for the polling worker.
    Assignment(WorkerAssignment),
    /// Heartbeat merged (always succeeds; unknown workers are ignored).
    Observed,
    /// Cluster-wide snapshot.
    Cluster(ClusterSnapshot),
    /// One worker's view, or `None` when undeclared.
    Worker(Option<WorkerStatusView>),
}

/// Node failures with operator-facing messages (mapped to Cap'n Proto
/// errors by the server in T04).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeError {
    /// Publish commit hash was empty or whitespace-only.
    EmptyCommit,
    /// Proposed generation is not newer than the active one.
    StaleGeneration {
        /// Generation the publisher proposed.
        proposed: Generation,
        /// Currently active generation.
        active: Generation,
    },
    /// Explicit-map scheduling failed (undeclared target).
    Schedule(ScheduleError),
}

impl Display for NodeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyCommit => f.write_str("publish commit must not be empty"),
            Self::StaleGeneration { proposed, active } => write!(
                f,
                "stale generation {proposed}: active generation is {active}"
            ),
            Self::Schedule(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for NodeError {}

/// Failures sending a request to the node or awaiting its reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestError {
    /// The node task is gone; the message could not be sent.
    Send,
    /// The node dropped the reply without answering.
    ReplyClosed,
}

impl Display for RequestError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Send => f.write_str("control plane node is not running"),
            Self::ReplyClosed => f.write_str("control plane node dropped the reply"),
        }
    }
}

impl std::error::Error for RequestError {}

/// Result of one node request: the typed reply or a node failure.
pub type NodeResult = Result<NodeReply, NodeError>;

/// Channel message pairing an event with its one-shot reply sender.
#[derive(Debug)]
pub struct NodeMessage {
    event: NodeEvent,
    sender: oneshot::Sender<NodeResult>,
}

impl NodeMessage {
    /// Creates a message and the receiver awaiting its reply.
    #[must_use]
    pub fn new(event: NodeEvent) -> (NodeReceiver, Self) {
        let (sender, rx) = oneshot::channel();
        let msg = Self { event, sender };
        (NodeReceiver(rx), msg)
    }

    /// Borrows the event.
    #[must_use]
    pub fn event(&self) -> &NodeEvent {
        &self.event
    }

    /// Answers the message, dropping the reply when the requester is gone.
    pub fn reply(self, result: NodeResult) {
        let _ = self.sender.send(result);
    }
}

/// Receiver awaiting one node's reply.
#[derive(Debug)]
pub struct NodeReceiver(Receiver<NodeResult>);

impl NodeReceiver {
    /// Awaits the node's reply.
    ///
    /// # Errors
    ///
    /// Returns the underlying `RecvError` when the node dropped the reply
    /// without answering.
    pub async fn recv(self) -> Result<NodeResult, oneshot::error::RecvError> {
        self.0.await
    }
}

/// Cloneable sender half used by the server to talk to the node.
#[derive(Debug, Clone)]
pub struct NodeMessenger(Sender<NodeMessage>);

impl NodeMessenger {
    /// Sends an event and awaits the node's typed reply.
    ///
    /// The outer `Result` reports transport health; an `Ok(Err(_))` inner
    /// value is the node's business-logic rejection (mapped to Cap'n Proto
    /// errors by the server in T04).
    ///
    /// # Errors
    ///
    /// Returns `RequestError::Send` when the node task is gone, or
    /// `RequestError::ReplyClosed` when it dropped the reply.
    pub async fn request(&self, event: NodeEvent) -> Result<NodeResult, RequestError> {
        let (receiver, message) = NodeMessage::new(event);
        self.0.send(message).await.map_err(|_| RequestError::Send)?;
        receiver.recv().await.map_err(|_| RequestError::ReplyClosed)
    }
}

impl From<Sender<NodeMessage>> for NodeMessenger {
    fn from(value: Sender<NodeMessage>) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{NodeError, RequestError};
    use crate::{scheduler::ScheduleError, store::Generation};

    use crate::registry::WorkerId;

    #[test]
    fn error_messages_name_the_offending_values() {
        assert_eq!(
            format!("{}", NodeError::EmptyCommit),
            "publish commit must not be empty"
        );
        assert_eq!(
            format!(
                "{}",
                NodeError::StaleGeneration {
                    proposed: Generation::new(1),
                    active: Generation::new(2),
                }
            ),
            "stale generation 1: active generation is 2"
        );
        assert_eq!(
            format!(
                "{}",
                NodeError::Schedule(ScheduleError::UnknownWorker(
                    WorkerId::new("ghost").expect("valid")
                ))
            ),
            "desired VM targets undeclared worker: ghost"
        );
        assert_eq!(
            format!("{}", RequestError::Send),
            "control plane node is not running"
        );
        assert_eq!(
            format!("{}", RequestError::ReplyClosed),
            "control plane node dropped the reply"
        );
    }
}
