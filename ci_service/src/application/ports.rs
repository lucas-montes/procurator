//! Ports: what the use cases need from the outside world.
//! Why: the run store, the builder and the clock are swappable (`SQLite` or
//! memory, the nix daemon or a VM later) without touching a use case.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::domain::access::{Credential, Principal};
use crate::domain::check::{Check, CheckId, CheckStatus, Derivation, NewCheck, Systems};
use crate::domain::event::{Delivery, EventPayload, Target};
use crate::domain::log::{LogLine, LogOrigin, NewLogLine};
use crate::domain::repo_config::{CheckSelection, RepoSettings, RunSettings, Timeouts};
use crate::domain::run::{GitRef, NewRun, RepoName, Run, RunId, RunStatus};

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

    /// Move every running run back to queued, dropping the checks and log
    /// lines of the interrupted attempt. Returns how many.
    /// Why: called at startup, when no worker can be running yet.
    fn requeue_running(&self) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// End a running run. `false` when the run is missing or not running.
    fn finish(
        &self,
        id: RunId,
        status: RunStatus,
        now: i64,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send {
        self.finish_with_outbox(id, status, now, &[], None)
    }

    /// End a running run and queue `outbox` deliveries, atomically: either
    /// both happen or neither. With `keep_logs_until`, its log lines are
    /// deleted by [`RunStore::delete_expired_logs`] from then on. `false`
    /// (and nothing queued) when the run is missing or not running.
    fn finish_with_outbox(
        &self,
        id: RunId,
        status: RunStatus,
        now: i64,
        outbox: &[(Target, EventPayload)],
        keep_logs_until: Option<i64>,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Delete the log lines (and derivation links) of every run whose logs
    /// were kept until `now` or earlier. Runs and checks stay. Returns how
    /// many lines.
    fn delete_expired_logs(&self, now: i64)
    -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// Queue deliveries due now.
    fn queue_deliveries(
        &self,
        outbox: &[(Target, EventPayload)],
        now: i64,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Up to `limit` pending deliveries due at `now`, oldest first.
    fn due_deliveries(
        &self,
        now: i64,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<Delivery>, StoreError>> + Send;

    /// Record one delivery attempt and what follows from it.
    fn record_attempt(
        &self,
        id: i64,
        next: NextAttempt,
        error: Option<&str>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// The status of the newest finished, non-errored run of `repo` on
    /// `git_ref` older than `before`.
    /// Why: "recovered" compares against the repo's last verdict, not an
    /// infrastructure fault.
    fn previous_status<'a>(
        &'a self,
        repo: &'a RepoName,
        git_ref: &'a GitRef,
        before: RunId,
    ) -> impl Future<Output = Result<Option<RunStatus>, StoreError>> + Send + 'a;

    /// One run.
    fn run(&self, id: RunId) -> impl Future<Output = Result<Option<Run>, StoreError>> + Send;

    /// A page of runs, newest first.
    fn runs(
        &self,
        query: RunQuery<'_>,
    ) -> impl Future<Output = Result<Vec<Run>, StoreError>> + Send;

    /// Store a run's evaluated checks, in order, and return them with ids.
    fn insert_checks(
        &self,
        run: RunId,
        checks: Vec<NewCheck>,
    ) -> impl Future<Output = Result<Vec<Check>, StoreError>> + Send;

    /// Move a check that is not over yet to `status`, recording `machine`
    /// when given. `false` when the check is missing or already over.
    fn update_check(
        &self,
        id: CheckId,
        status: CheckStatus,
        machine: Option<&str>,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Queue a failed check for another try: back to queued, one more try,
    /// no machine. `false` when it is missing or did not fail.
    fn retry_check(&self, id: CheckId) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Move every check of a run that is not over yet to `status`.
    /// Returns how many.
    fn close_open_checks(
        &self,
        run: RunId,
        status: CheckStatus,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// A run's checks, in evaluation order.
    fn checks(&self, run: RunId) -> impl Future<Output = Result<Vec<Check>, StoreError>> + Send;

    /// Record that a check needs these derivations, so their build logs
    /// show with its own. Recording a link twice is not an error.
    fn link_derivations(
        &self,
        links: &[(CheckId, &str)],
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// The derivations linked to a check, in no particular order.
    fn linked_derivations(
        &self,
        check: CheckId,
    ) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send;

    /// Append lines to a run's log, in order. Returns the sequence number
    /// ([`LogLine::seq`]) of the first; the others follow it one by one.
    fn append_log(
        &self,
        run: RunId,
        lines: &[NewLogLine<'_>],
    ) -> impl Future<Output = Result<i64, StoreError>> + Send;

    /// A run's log lines from the given origins, in order; every line when
    /// `origins` is empty.
    fn log(
        &self,
        run: RunId,
        origins: &[LogOrigin<'_>],
    ) -> impl Future<Output = Result<Vec<LogLine>, StoreError>> + Send;
}

/// Why a builder could not do its job.
#[derive(Debug)]
pub enum BuilderError {
    /// The repository's flake could not be evaluated at all (no `flake.nix`,
    /// a syntax error, a broken lock file). The repo's fault: the run fails.
    Flake(String),
    /// The builder itself is broken (Nix missing, daemon down, repo
    /// missing on disk). Not the repo's fault: the run errors.
    Unavailable(String),
}

impl fmt::Display for BuilderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Flake(detail) => write!(f, "flake evaluation failed: {detail}"),
            Self::Unavailable(detail) => write!(f, "builder unavailable: {detail}"),
        }
    }
}

impl std::error::Error for BuilderError {}

/// One evaluated flake attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evaluated {
    /// It is a derivation.
    Derivation {
        /// e.g. `checks.x86_64-linux.unit`
        attr: String,
        /// The derivation's system.
        system: String,
        /// The derivation.
        derivation: Derivation,
        /// Its outputs already exist in the cache or on the builder.
        cached: bool,
    },
    /// Evaluating it failed.
    Failed {
        /// e.g. `checks.x86_64-linux.unit`
        attr: String,
        /// The system named by the attribute path.
        system: String,
        /// Nix's error message.
        error: String,
    },
}

/// What a builder reports while building.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildEvent {
    /// A derivation started building. `build` tags the lines that follow.
    Started {
        /// Tag for this build's [`BuildEvent::Line`]s.
        build: u64,
        /// The `.drv` path.
        drv: String,
        /// [`crate::domain::check::LOCAL_MACHINE`] or a remote builder.
        machine: String,
    },
    /// One line of a build's output.
    Line {
        /// The [`BuildEvent::Started`] tag.
        build: u64,
        /// The line.
        text: String,
    },
    /// The build tagged `build` ran into a time limit.
    TimedOut {
        /// The [`BuildEvent::Started`] tag.
        build: u64,
    },
    /// A message about the whole build (e.g. why a derivation failed).
    Message(String),
}

/// Evaluates and builds flakes.
pub trait Builder: Send + Sync + 'static {
    /// The systems the CI host or one of its remote builders can build.
    fn systems(&self) -> impl Future<Output = Result<Systems, BuilderError>> + Send;

    /// Evaluate the selected attributes of a run's commit and, when
    /// `deployment`, the flake's
    /// [`DEPLOYMENT_ATTR`](crate::domain::deploy::DEPLOYMENT_ATTR) as one
    /// more attribute of that name (an error entry when it is missing).
    fn evaluate(
        &self,
        run: &NewRun,
        selection: &CheckSelection,
        deployment: bool,
    ) -> impl Future<Output = Result<Vec<Evaluated>, BuilderError>> + Send;

    /// The contents of the file at store path `path`, fetched from a
    /// substituter first if this host lacks it. At most `limit` bytes.
    /// Why: a cached deployment was never built here.
    fn read_output(
        &self,
        path: &str,
        limit: u64,
    ) -> impl Future<Output = Result<String, BuilderError>> + Send;

    /// The derivations `drv` needs built first, all the way down, without
    /// `drv` itself.
    /// Why: a dependency built in the run shows in each check's log.
    fn requisites(
        &self,
        drv: &Derivation,
    ) -> impl Future<Output = Result<Vec<String>, BuilderError>> + Send;

    /// Build `targets` in one go, keeping on after failures, under the
    /// per-derivation limits of `timeouts`, and send progress to `events`.
    /// Returns, per target in order, whether all its outputs now exist.
    fn build(
        &self,
        targets: &[&Derivation],
        timeouts: Timeouts,
        events: mpsc::Sender<BuildEvent>,
    ) -> impl Future<Output = Result<Vec<bool>, BuilderError>> + Send;

    /// Upload to `cache` the outputs of `drvs` (derivations this run built)
    /// that exist, plus the paths they reference that were created here.
    /// Never their downloaded dependencies. Returns how many paths.
    fn push(
        &self,
        drvs: &[&str],
        cache: &str,
    ) -> impl Future<Output = Result<usize, BuilderError>> + Send;
}

/// Why a repo config could not be read.
#[derive(Debug)]
pub enum RepoConfigError {
    /// `procurator.nix` is invalid. The repo's fault: the run fails.
    Invalid(String),
    /// Git or Nix could not be run. Not the repo's fault: the run errors.
    Unavailable(String),
}

impl fmt::Display for RepoConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(detail) => detail.fmt(f),
            Self::Unavailable(detail) => write!(f, "repo config unavailable: {detail}"),
        }
    }
}

impl std::error::Error for RepoConfigError {}

/// Reads `procurator.nix`.
pub trait ConfigSource: Send + Sync + 'static {
    /// The per-run settings of a run's commit, before the service's caps.
    /// No `procurator.nix` means the defaults.
    fn run_settings(
        &self,
        run: &NewRun,
    ) -> impl Future<Output = Result<RunSettings, RepoConfigError>> + Send;

    /// The repo-level settings, from the head of the default branch.
    /// No `procurator.nix` there (or no default branch) means the defaults.
    fn repo_settings(
        &self,
        repo: &RepoName,
    ) -> impl Future<Output = Result<RepoSettings, RepoConfigError>> + Send;
}

/// What happens to a delivery after an attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextAttempt {
    /// It arrived.
    Delivered,
    /// Try again at this time (unix seconds).
    RetryAt(i64),
    /// Stop trying.
    GiveUp,
}

/// Why a delivery failed.
#[derive(Debug)]
pub struct DeliveryError(String);

impl DeliveryError {
    /// Wrap a cause.
    pub fn new(cause: impl fmt::Display) -> Self {
        Self(cause.to_string())
    }
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for DeliveryError {}

/// Delivers events to their targets. Each adapter has its own retries.
pub trait Notifier: Send + Sync + 'static {
    /// Send one event to one target.
    fn deliver(
        &self,
        target: &Target,
        payload: &EventPayload,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send;

    /// How long to wait before attempt `attempts + 1`, after `attempts`
    /// failed ones; `None` to give up.
    fn retry_delay(&self, target: &Target, attempts: u32) -> Option<Duration>;
}

/// Resolves secret names (e.g. webhook URLs) to their values.
pub trait SecretStore: Send + Sync + 'static {
    /// The secret's value, `None` when there is no such secret.
    fn secret(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<Option<String>, DeliveryError>> + Send;
}

/// Why a caller's access could not be decided.
#[derive(Debug)]
pub enum AccessError {
    /// The credential is invalid or expired.
    Unauthenticated,
    /// The access backend could not answer.
    Unavailable(String),
}

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthenticated => f.write_str("invalid credential"),
            Self::Unavailable(detail) => write!(f, "access check unavailable: {detail}"),
        }
    }
}

impl std::error::Error for AccessError {}

/// Who may read which repo's runs and logs.
/// Why: the repo host (repohub) owns users and permissions, not CI.
pub trait Access: Send + Sync + 'static {
    /// Who presented `credential`. No credential is not an error: it is
    /// [`Principal::Anonymous`], which may still read public repos.
    fn authenticate(
        &self,
        credential: Credential<'_>,
    ) -> impl Future<Output = Result<Principal, AccessError>> + Send;

    /// Whether `principal` may read `repo`'s runs and logs.
    fn can_read(
        &self,
        principal: &Principal,
        repo: &RepoName,
    ) -> impl Future<Output = Result<bool, AccessError>> + Send;
}

/// Wall clock.
pub trait Clock: Send + Sync + 'static {
    /// Current time, unix seconds.
    fn now(&self) -> i64;
}
