//! Ports: what the use cases need from the outside world.
//! Why: the run store and the clock are swappable (`SQLite` or memory today,
//! another service's database when deployed as a monolith) without touching
//! a use case.

use std::fmt;
use std::future::Future;

use crate::domain::run::{NewRun, RepoName, Run, RunId, RunStatus};

/// Run store failure (the backend's message).
#[derive(Debug)]
pub struct StoreError(String);

impl StoreError {
    /// Wrap a backend error.
    pub fn new(cause: impl fmt::Display) -> Self {
        Self(cause.to_string())
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "run store: {}", self.0)
    }
}

impl std::error::Error for StoreError {}

/// Which runs to list: newest first, optionally one repo, paged by id.
#[derive(Debug, Clone, Copy)]
pub struct RunQuery<'a> {
    repo: Option<&'a RepoName>,
    before: Option<RunId>,
    limit: u32,
}

impl<'a> RunQuery<'a> {
    /// Describe a page of runs.
    #[must_use]
    pub fn new(repo: Option<&'a RepoName>, before: Option<RunId>, limit: u32) -> Self {
        Self {
            repo,
            before,
            limit,
        }
    }

    /// Only runs of this repo, when set.
    #[must_use]
    pub fn repo(&self) -> Option<&'a RepoName> {
        self.repo
    }

    /// Only runs with a smaller id, when set.
    #[must_use]
    pub fn before(&self) -> Option<RunId> {
        self.before
    }

    /// At most this many runs.
    #[must_use]
    pub fn limit(&self) -> u32 {
        self.limit
    }
}

/// The source of truth for runs. Times are unix seconds.
/// Every state change must be atomic: two workers never claim one run.
pub trait RunStore: Send + Sync + 'static {
    /// Store a new queued run and return its id.
    fn insert(
        &self,
        run: NewRun,
        now: i64,
    ) -> impl Future<Output = Result<RunId, StoreError>> + Send;

    /// Atomically move the oldest queued run to running and return it.
    fn claim_next(&self, now: i64) -> impl Future<Output = Result<Option<Run>, StoreError>> + Send;

    /// Move every running run back to queued. Returns how many.
    /// Why: called at startup, when no worker can be running yet.
    fn requeue_running(&self) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// End a running run. `false` when the run is missing or not running.
    fn finish(
        &self,
        id: RunId,
        status: RunStatus,
        now: i64,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// One run.
    fn run(&self, id: RunId) -> impl Future<Output = Result<Option<Run>, StoreError>> + Send;

    /// A page of runs, newest first.
    fn runs(
        &self,
        query: RunQuery<'_>,
    ) -> impl Future<Output = Result<Vec<Run>, StoreError>> + Send;
}

/// Wall clock.
pub trait Clock: Send + Sync + 'static {
    /// Current time, unix seconds.
    fn now(&self) -> i64;
}
