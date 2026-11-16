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
    Access, BuildEvent, Builder, BuilderError, Clock, ConfigSource, Evaluated, Notifier,
    RepoConfigError, RunStore, StoreError,
};
use crate::domain::check::{Check, CheckId, CheckStatus, LOCAL_MACHINE, NewCheck};
use crate::domain::deploy::{DEPLOYMENT_ATTR, Deployment, MAX_DEPLOYMENT_BYTES};
use crate::domain::event::{EventKind, EventPayload, deliveries, deploy_delivery};
use crate::domain::log::{LogOrigin, NewLogLine};
use crate::domain::repo_config::{RepoSettings, RunSettings, Timeouts};
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

/// How a run ended, and what it deploys.
struct Verdict {
    status: RunStatus,
    deployment: Option<Deployment>,
}

impl Verdict {
    fn without_deploy(status: RunStatus) -> Self {
        Self {
            status,
            deployment: None,
        }
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

impl<S, C, B, R, N, A> Service<S, C, B, R, N, A>
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
    N: Notifier,
    A: Access,
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
            let result = self.carry_out(&run).await;
            // Why: whatever happened, readers following the run see its end.
            self.live.close(run.id());
            result.map(|()| true)
        }
    }

    /// Carry out a claimed run, up to finishing it.
    async fn carry_out(&self, run: &Run) -> Result<(), ServiceError> {
        let id = run.id();
        tracing::info!(run = %id, repo = run.request().repo().as_str(), "run started");
        let repo = self.repo_settings(run).await?;
        self.announce(id, EventKind::RunStarted, RunStatus::Running, &repo)
            .await?;
        let verdict = match self.execute(run, &repo).await {
            Ok(verdict) => verdict,
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
                Verdict::without_deploy(status)
            }
        };
        let status = verdict.status;
        self.conclude(run, verdict, &repo).await?;
        tracing::info!(run = %id, status = status.as_str(), "run finished");
        Ok(())
    }

    /// The repo-level settings from the default branch. A broken config
    /// there is a warning in this run's log, and the defaults apply.
    async fn repo_settings(&self, run: &Run) -> Result<RepoSettings, StoreError> {
        match self.config.repo_settings(run.request().repo()).await {
            Ok(settings) => Ok(settings),
            Err(error) => {
                let message = format!("warning: default branch config ignored: {error}");
                self.log_text(run.id(), LogOrigin::Run, &message).await?;
                Ok(RepoSettings::default())
            }
        }
    }

    /// Finish the run and queue its events for the webhooks that want
    /// them, and its deployment for the deploy target, in one store
    /// transaction.
    async fn conclude(
        &self,
        run: &Run,
        verdict: Verdict,
        repo: &RepoSettings,
    ) -> Result<(), ServiceError> {
        let id = run.id();
        let request = run.request();
        let status = verdict.status;
        let deploy = repo.deploy().zip(verdict.deployment);
        let outbox = if repo.webhooks().is_empty() && deploy.is_none() {
            Vec::new()
        } else {
            let checks = self.store.checks(id).await?;
            let previous = self
                .store
                .previous_status(request.repo(), request.git_ref(), id)
                .await?;
            let on_default = repo.default_ref() == Some(request.git_ref().as_str());
            let events = EventPayload::for_finished(run, status, &checks, previous, on_default);
            let mut outbox = deliveries(&events, repo.webhooks());
            if let Some((settings, deployment)) = deploy {
                outbox.extend(deploy_delivery(&events, settings, deployment));
            }
            outbox
        };
        let now = self.clock.now();
        let keep_until = now + self.retention.seconds(repo.log_retention_days());
        let finished = self
            .store
            .finish_with_outbox(id, status, now, &outbox, Some(keep_until))
            .await?;
        if finished {
            Ok(())
        } else {
            Err(ServiceError::Conflict(RunStatus::Running))
        }
    }

    /// Read the config, then check within the run's time limit.
    async fn execute(&self, run: &Run, repo: &RepoSettings) -> Result<Verdict, Fault> {
        let settings = self
            .config
            .run_settings(run.request())
            .await?
            .capped(self.limits);
        let limit = Duration::from_secs(settings.timeouts().run());
        // Why: dropping the check future on timeout kills its Nix processes.
        if let Ok(result) = tokio::time::timeout(limit, self.check(run, &settings, repo)).await {
            return result;
        }
        let id = run.id();
        self.store
            .close_open_checks(id, CheckStatus::TimedOut)
            .await?;
        let message = format!("run timed out after {} seconds", settings.timeouts().run());
        self.log_text(id, LogOrigin::Run, &message).await?;
        Ok(Verdict::without_deploy(RunStatus::TimedOut))
    }

    /// Evaluate, classify, build; the run's final status, and what it
    /// deploys when it passed on the deploy branch.
    async fn check(
        &self,
        run: &Run,
        settings: &RunSettings,
        repo: &RepoSettings,
    ) -> Result<Verdict, Fault> {
        let id = run.id();
        let deploys = repo
            .deploy()
            .is_some_and(|deploy| deploy.applies_to(run.request().git_ref()));
        let systems = self.builder.systems().await?;
        let evaluated = self
            .builder
            .evaluate(run.request(), settings.checks(), deploys)
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
        let requisites = self.requisites(id, &queued).await?;
        let mut dependents: HashMap<&str, Vec<CheckId>> = HashMap::new();
        for (check, drvs) in &requisites {
            for drv in drvs {
                dependents.entry(drv.as_str()).or_default().push(*check);
            }
        }
        let mut built = Vec::new();
        finals.extend(
            self.build_with_reruns(id, queued, settings, &dependents, &mut built)
                .await?,
        );
        if !built.is_empty() {
            self.push(run, &built, repo).await?;
        }
        let status = CheckStatus::run_outcome(finals);
        if !deploys || status != RunStatus::Passed {
            return Ok(Verdict::without_deploy(status));
        }
        match self.deployment(id).await? {
            Ok(deployment) => {
                let message = format!(
                    "deploying {} vms from {}",
                    deployment.vms().len(),
                    deployment.path()
                );
                self.log_text(id, LogOrigin::Run, &message).await?;
                Ok(Verdict {
                    status,
                    deployment: Some(deployment),
                })
            }
            Err(reason) => {
                self.log_text(id, LogOrigin::Run, &reason).await?;
                Ok(Verdict::without_deploy(RunStatus::Failed))
            }
        }
    }

    /// The deployment a passed run built, or why it is unusable (the
    /// repo's fault: the run fails).
    async fn deployment(&self, run: RunId) -> Result<Result<Deployment, String>, Fault> {
        let checks = self.store.checks(run).await?;
        let Some(check) = checks.iter().find(|check| check.attr() == DEPLOYMENT_ATTR) else {
            return Ok(Err(format!("deploy: {DEPLOYMENT_ATTR} was not evaluated")));
        };
        let output = check.derivation().and_then(|drv| drv.outputs().first());
        let path = match (check.status(), output) {
            (CheckStatus::Passed | CheckStatus::Cached, Some(path)) => path,
            (status, _) => {
                return Ok(Err(format!(
                    "deploy: {DEPLOYMENT_ATTR} is {}, so there is nothing to deploy",
                    status.as_str()
                )));
            }
        };
        let json = match self.builder.read_output(path, MAX_DEPLOYMENT_BYTES).await {
            Ok(json) => json,
            Err(BuilderError::Flake(reason)) => return Ok(Err(format!("deploy: {reason}"))),
            Err(error) => return Err(Fault::Builder(error)),
        };
        Ok(Deployment::parse(String::from(path.as_str()), &json)
            .map_err(|reason| format!("deploy: {reason}")))
    }

    /// What each queued check needs built first. A failure only costs that
    /// check its dependency logs, with a warning in its log.
    async fn requisites(
        &self,
        run: RunId,
        queued: &[&Check],
    ) -> Result<Vec<(CheckId, Vec<String>)>, StoreError> {
        let mut requisites = Vec::with_capacity(queued.len());
        for check in queued {
            let Some(drv) = check.derivation() else {
                continue;
            };
            match self.builder.requisites(drv).await {
                Ok(drvs) => requisites.push((check.id(), drvs)),
                Err(error) => {
                    let message = format!("warning: dependency logs not attached: {error}");
                    self.log_text(run, LogOrigin::Check(check.attr()), &message)
                        .await?;
                }
            }
        }
        Ok(requisites)
    }

    /// Upload what this run built to the repo's cache. Failures are logged
    /// and never fail the run.
    /// Why: the outputs are already checked; the cache only makes the next
    /// run and deploys faster.
    async fn push(
        &self,
        run: &Run,
        built: &[String],
        settings: &RepoSettings,
    ) -> Result<(), StoreError> {
        let Some(policy) = &self.cache else {
            return Ok(());
        };
        let id = run.id();
        let repo = run.request().repo();
        let message = match policy.choose(repo.owner(), settings.cache()) {
            Err(error) => format!("warning: not pushed to a cache: {error}"),
            Ok(cache) => {
                let drvs: Vec<&str> = built.iter().map(String::as_str).collect();
                match self.builder.push(&drvs, &cache).await {
                    Ok(paths) => format!("pushed {paths} paths to cache {cache}"),
                    Err(error) => format!("warning: push to cache {cache} failed: {error}"),
                }
            }
        };
        self.log_text(id, LogOrigin::Run, &message).await
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
        self.append(run, &lines).await
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
        dependents: &HashMap<&str, Vec<CheckId>>,
        built: &mut Vec<String>,
    ) -> Result<Vec<CheckStatus>, Fault> {
        let reruns = settings.reruns();
        let mut finals = Vec::with_capacity(pending.len());
        for attempt in 0..=reruns {
            if pending.is_empty() {
                break;
            }
            let statuses = self
                .build(run, &pending, settings.timeouts(), dependents, built)
                .await?;
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
            self.append(run, &lines).await?;
            pending = failed;
        }
        built.sort_unstable();
        built.dedup();
        Ok(finals)
    }

    /// Build checks once while recording progress; their statuses, in order.
    async fn build(
        &self,
        run: RunId,
        queued: &[&Check],
        timeouts: Timeouts,
        dependents: &HashMap<&str, Vec<CheckId>>,
        started: &mut Vec<String>,
    ) -> Result<Vec<CheckStatus>, Fault> {
        let targets: Vec<_> = queued
            .iter()
            .filter_map(|check| check.derivation())
            .collect();
        let (events, received) = mpsc::channel(EVENT_BUFFER);
        let (built, recorded) = tokio::join!(
            self.builder.build(&targets, timeouts, events),
            self.record(run, queued, received, dependents, started)
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
    /// derivation, checks marked `building` on the machine that took them,
    /// each dependency linked to the checks that need it (before its first
    /// line, so live readers of those checks see it). Returns the checks
    /// whose build hit a time limit, and adds every derivation that started
    /// building to `started`.
    async fn record(
        &self,
        run: RunId,
        queued: &[&Check],
        mut received: mpsc::Receiver<BuildEvent>,
        dependents: &HashMap<&str, Vec<CheckId>>,
        started: &mut Vec<String>,
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
                    if let Some(needers) = dependents.get(drv.as_str()) {
                        let links: Vec<(CheckId, &str)> =
                            needers.iter().map(|&check| (check, drv.as_str())).collect();
                        self.store.link_derivations(&links).await?;
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
            self.append(run, &lines).await?;
            batch.clear();
        }
        started.extend(builds.into_values());
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
        self.append(run, &lines).await
    }
}
