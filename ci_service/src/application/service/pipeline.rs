//! The worker side: claim a run, read its repo config, evaluate its checks,
//! build what is neither cached nor skipped (again for failures, up to the
//! reruns allowed), record per-derivation logs, finish the run.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::mem;
use std::time::Duration;

use tokio::sync::mpsc;

use super::Service;
use crate::application::ServiceError;
use crate::application::ports::{
    BuildEvent, Builder, BuilderError, Clock, ConfigSource, Evaluated, RepoConfigError, RunStore,
    StoreError,
};
use crate::domain::check::{Check, CheckId, CheckStatus, LOCAL_MACHINE, NewCheck};
use crate::domain::log::{LogOrigin, NewLogLine};
use crate::domain::repo_config::{RunSettings, Timeouts};
use crate::domain::run::{Run, RunId, RunStatus};

/// Build events buffered between the builder and the log writer.
/// Why: bounded, so a chatty build waits for the store instead of filling
/// memory.
const EVENT_BUFFER: usize = 1024;
/// Events written to the store per batch.
const EVENT_BATCH: usize = 256;

/// Why a run could not be carried out.
enum Fault {
    Config(RepoConfigError),
    Builder(BuilderError),
    Store(StoreError),
}

impl From<RepoConfigError> for Fault {
    fn from(error: RepoConfigError) -> Self {
        Self::Config(error)
    }
}

impl From<BuilderError> for Fault {
    fn from(error: BuilderError) -> Self {
        Self::Builder(error)
    }
}

impl From<StoreError> for Fault {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl Fault {
    /// The run's status for a fault that is not the store's, and why.
    /// Why: a broken repo (bad config, unevaluable flake) fails the run; a
    /// broken host errors it.
    fn outcome(&self) -> Option<RunStatus> {
        match self {
            Self::Config(RepoConfigError::Invalid(_)) | Self::Builder(BuilderError::Flake(_)) => {
                Some(RunStatus::Failed)
            }
            Self::Config(RepoConfigError::Unavailable(_))
            | Self::Builder(BuilderError::Unavailable(_)) => Some(RunStatus::Errored),
            Self::Store(_) => None,
        }
    }

    fn message(&self) -> String {
        match self {
            Self::Config(error) => error.to_string(),
            Self::Builder(error) => error.to_string(),
            Self::Store(error) => error.to_string(),
        }
    }
}

impl<S, C, B, R> Service<S, C, B, R>
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
{
    /// Work runs off the queue forever, waiting `poll` whenever it is empty
    /// or the store fails. Drop the future to stop; a run cut off this way
    /// stays `running` and is requeued by [`Service::recover`] at the next
    /// start.
    pub fn work(&self, poll: Duration) -> impl Future<Output = ()> + Send + '_ {
        async move {
            loop {
                match self.work_next().await {
                    Ok(true) => {}
                    Ok(false) => tokio::time::sleep(poll).await,
                    Err(error) => {
                        tracing::error!(%error, "worker failed");
                        tokio::time::sleep(poll).await;
                    }
                }
            }
        }
    }

    /// Claim the oldest queued run and carry it out. `false` when nothing
    /// was queued.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Internal`] when the store fails. The run is then
    /// marked `errored` if the store still allows it.
    pub fn work_next(&self) -> impl Future<Output = Result<bool, ServiceError>> + Send + '_ {
        async move {
            let Some(run) = self.claim_next().await? else {
                return Ok(false);
            };
            let id = run.id();
            tracing::info!(run = %id, repo = run.request().repo().as_str(), "run started");
            let result = self.execute(&run).await;
            let status = match result {
                Ok(status) => status,
                Err(fault) => {
                    let Some(status) = fault.outcome() else {
                        // Why: best effort, so the run does not look stuck.
                        let _ = self.finish(id, RunStatus::Errored).await;
                        return Err(ServiceError::Internal(fault.message()));
                    };
                    // Why: checks the broken builder never finished must not
                    // look like they are still building.
                    self.store
                        .close_open_checks(id, CheckStatus::Failed)
                        .await?;
                    self.log_text(id, LogOrigin::Run, &fault.message()).await?;
                    status
                }
            };
            self.finish(id, status).await?;
            tracing::info!(run = %id, status = status.as_str(), "run finished");
            Ok(true)
        }
    }

    /// Read the config, then check within the run's time limit.
    async fn execute(&self, run: &Run) -> Result<RunStatus, Fault> {
        let settings = self
            .config
            .run_settings(run.request())
            .await?
            .capped(self.limits);
        let limit = Duration::from_secs(settings.timeouts().run());
        // Why: dropping the check future on timeout kills its Nix processes.
        if let Ok(result) = tokio::time::timeout(limit, self.check(run, &settings)).await {
            return result;
        }
        let id = run.id();
        self.store
            .close_open_checks(id, CheckStatus::TimedOut)
            .await?;
        let message = format!("run timed out after {} seconds", settings.timeouts().run());
        self.log_text(id, LogOrigin::Run, &message).await?;
        Ok(RunStatus::TimedOut)
    }

    /// Evaluate, classify, build; the run's final status.
    async fn check(&self, run: &Run, settings: &RunSettings) -> Result<RunStatus, Fault> {
        let id = run.id();
        let systems = self.builder.systems().await?;
        let evaluated = self
            .builder
            .evaluate(run.request(), settings.checks())
            .await?;

        let mut errors = Vec::with_capacity(evaluated.len());
        let new: Vec<NewCheck> = evaluated
            .into_iter()
            .map(|evaluated| match evaluated {
                Evaluated::Derivation {
                    attr,
                    system,
                    derivation,
                    cached,
                } => {
                    errors.push(None);
                    NewCheck::evaluated(attr, system, derivation, cached, &systems)
                }
                Evaluated::Failed {
                    attr,
                    system,
                    error,
                } => {
                    errors.push(Some(error));
                    NewCheck::eval_failed(attr, system)
                }
            })
            .collect();
        let checks = self.store.insert_checks(id, new).await?;
        self.log_evaluation(id, &checks, &errors).await?;

        let queued: Vec<&Check> = checks
            .iter()
            .filter(|check| check.status() == CheckStatus::Queued)
            .collect();
        let mut finals: Vec<CheckStatus> = checks
            .iter()
            .filter(|check| check.status() != CheckStatus::Queued)
            .map(Check::status)
            .collect();
        finals.extend(self.build_with_reruns(id, queued, settings).await?);
        Ok(CheckStatus::run_outcome(finals))
    }

    /// Store why checks failed to evaluate or were skipped.
    async fn log_evaluation(
        &self,
        run: RunId,
        checks: &[Check],
        errors: &[Option<String>],
    ) -> Result<(), StoreError> {
        let skipped: Vec<String> = checks
            .iter()
            .filter(|check| check.status() == CheckStatus::Skipped)
            .map(|check| format!("skipped: no builder for {}", check.system()))
            .collect();
        let mut skipped = skipped.iter();
        let mut lines = Vec::new();
        for (check, error) in checks.iter().zip(errors) {
            let origin = LogOrigin::Check(check.attr());
            if let Some(error) = error {
                lines.extend(error.lines().map(|text| NewLogLine::new(origin, text)));
            } else if check.status() == CheckStatus::Skipped
                && let Some(reason) = skipped.next()
            {
                lines.push(NewLogLine::new(origin, reason));
            }
        }
        if lines.is_empty() {
            return Ok(());
        }
        self.store.append_log(run, &lines).await
    }

    /// Build the queued checks, then the failed ones again up to
    /// `reruns` times; every final status.
    /// Why: each try is stored, so a check that failed and then passed shows
    /// up as flaky (`passed` with more than one try).
    async fn build_with_reruns(
        &self,
        run: RunId,
        mut pending: Vec<&Check>,
        settings: &RunSettings,
    ) -> Result<Vec<CheckStatus>, Fault> {
        let reruns = settings.reruns();
        let mut finals = Vec::with_capacity(pending.len());
        for attempt in 0..=reruns {
            if pending.is_empty() {
                break;
            }
            let statuses = self.build(run, &pending, settings.timeouts()).await?;
            let mut failed = Vec::new();
            for (check, status) in pending.into_iter().zip(statuses) {
                if status == CheckStatus::Failed && attempt < reruns {
                    failed.push(check);
                } else {
                    finals.push(status);
                }
            }
            let try_number = attempt + 2;
            let total = reruns + 1;
            let notes: Vec<String> = failed
                .iter()
                .map(|_| format!("failed, building again: try {try_number} of {total}"))
                .collect();
            let mut lines = Vec::with_capacity(failed.len());
            for (check, note) in failed.iter().zip(&notes) {
                self.store.retry_check(check.id()).await?;
                lines.push(NewLogLine::new(LogOrigin::Check(check.attr()), note));
            }
            if !lines.is_empty() {
                self.store.append_log(run, &lines).await?;
            }
            pending = failed;
        }
        Ok(finals)
    }

    /// Build checks once while recording progress; their statuses, in order.
    async fn build(
        &self,
        run: RunId,
        queued: &[&Check],
        timeouts: Timeouts,
    ) -> Result<Vec<CheckStatus>, Fault> {
        let targets: Vec<_> = queued
            .iter()
            .filter_map(|check| check.derivation())
            .collect();
        let (events, received) = mpsc::channel(EVENT_BUFFER);
        let (built, recorded) = tokio::join!(
            self.builder.build(&targets, timeouts, events),
            self.record(run, queued, received)
        );
        let timed_out = recorded?;
        let built = built?;

        let mut statuses = Vec::with_capacity(queued.len());
        for (check, ok) in queued.iter().zip(built) {
            let status = if ok {
                CheckStatus::Passed
            } else if timed_out.contains(&check.id()) {
                CheckStatus::TimedOut
            } else {
                CheckStatus::Failed
            };
            self.store.update_check(check.id(), status, None).await?;
            statuses.push(status);
        }
        Ok(statuses)
    }

    /// Store build events until the builder is done: lines tagged with their
    /// derivation, checks marked `building` on the machine that took them.
    /// Returns the checks whose build hit a time limit.
    async fn record(
        &self,
        run: RunId,
        queued: &[&Check],
        mut received: mpsc::Receiver<BuildEvent>,
    ) -> Result<HashSet<CheckId>, StoreError> {
        let checks: HashMap<&str, CheckId> = queued
            .iter()
            .filter_map(|check| Some((check.derivation()?.path(), check.id())))
            .collect();
        let mut builds: HashMap<u64, String> = HashMap::new();
        let mut timed_out = HashSet::new();
        let mut batch = Vec::with_capacity(EVENT_BATCH);
        while received.recv_many(&mut batch, EVENT_BATCH).await > 0 {
            for event in &mut batch {
                if let BuildEvent::Started {
                    build,
                    drv,
                    machine,
                } = event
                {
                    if let Some(&check) = checks.get(drv.as_str()) {
                        let machine = if machine.is_empty() {
                            LOCAL_MACHINE
                        } else {
                            machine.as_str()
                        };
                        self.store
                            .update_check(check, CheckStatus::Building, Some(machine))
                            .await?;
                    }
                    builds.insert(*build, mem::take(drv));
                }
            }
            let mut lines = Vec::with_capacity(batch.len());
            for event in &batch {
                match event {
                    BuildEvent::Started { .. } => {}
                    BuildEvent::Line { build, text } => {
                        let origin = builds
                            .get(build)
                            .map_or(LogOrigin::Run, |drv| LogOrigin::Derivation(drv));
                        lines.push(NewLogLine::new(origin, text));
                    }
                    BuildEvent::TimedOut { build } => {
                        if let Some(drv) = builds.get(build) {
                            if let Some(&check) = checks.get(drv.as_str()) {
                                timed_out.insert(check);
                            }
                            let origin = LogOrigin::Derivation(drv);
                            lines.push(NewLogLine::new(origin, "error: build timed out"));
                        }
                    }
                    BuildEvent::Message(text) => lines.extend(
                        text.lines()
                            .map(|line| NewLogLine::new(LogOrigin::Run, line)),
                    ),
                }
            }
            if !lines.is_empty() {
                self.store.append_log(run, &lines).await?;
            }
            batch.clear();
        }
        Ok(timed_out)
    }

    /// Store a possibly multi-line message.
    async fn log_text(
        &self,
        run: RunId,
        origin: LogOrigin<'_>,
        text: &str,
    ) -> Result<(), StoreError> {
        let lines: Vec<_> = text
            .lines()
            .map(|line| NewLogLine::new(origin, line))
            .collect();
        self.store.append_log(run, &lines).await
    }
}
