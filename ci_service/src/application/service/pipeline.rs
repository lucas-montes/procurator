//! The worker side: claim a run, evaluate its checks, build what is neither
//! cached nor skipped, record per-derivation logs, finish the run.

use std::collections::HashMap;
use std::future::Future;
use std::mem;
use std::time::Duration;

use tokio::sync::mpsc;

use super::Service;
use crate::application::ServiceError;
use crate::application::ports::{
    BuildEvent, Builder, BuilderError, Clock, Evaluated, RunStore, StoreError,
};
use crate::domain::check::{Check, CheckId, CheckStatus, LOCAL_MACHINE, NewCheck};
use crate::domain::log::{LogOrigin, NewLogLine};
use crate::domain::run::{Run, RunId, RunStatus};

/// Build events buffered between the builder and the log writer.
/// Why: bounded, so a chatty build waits for the store instead of filling
/// memory.
const EVENT_BUFFER: usize = 1024;
/// Events written to the store per batch.
const EVENT_BATCH: usize = 256;

/// Why a run could not be carried out.
enum Fault {
    Builder(BuilderError),
    Store(StoreError),
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

impl<S, C, B> Service<S, C, B>
where
    S: RunStore,
    C: Clock,
    B: Builder,
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
            let status = match self.execute(&run).await {
                Ok(status) => status,
                Err(Fault::Builder(error)) => {
                    let status = match error {
                        BuilderError::Flake(_) => RunStatus::Failed,
                        BuilderError::Unavailable(_) => RunStatus::Errored,
                    };
                    let message = error.to_string();
                    self.log_text(id, LogOrigin::Run, &message).await?;
                    status
                }
                Err(Fault::Store(error)) => {
                    // Why: best effort, so the run does not look stuck.
                    let _ = self.finish(id, RunStatus::Errored).await;
                    return Err(error.into());
                }
            };
            self.finish(id, status).await?;
            tracing::info!(run = %id, status = status.as_str(), "run finished");
            Ok(true)
        }
    }

    /// Evaluate, classify, build; the run's final status.
    async fn execute(&self, run: &Run) -> Result<RunStatus, Fault> {
        let id = run.id();
        let systems = self.builder.systems().await?;
        let evaluated = self.builder.evaluate(run.request()).await?;

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
        if !queued.is_empty() {
            finals.extend(self.build(id, &queued).await?);
        }
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

    /// Build the queued checks while recording progress; their final
    /// statuses, in order.
    async fn build(&self, run: RunId, queued: &[&Check]) -> Result<Vec<CheckStatus>, Fault> {
        let targets: Vec<_> = queued
            .iter()
            .filter_map(|check| check.derivation())
            .collect();
        let (events, received) = mpsc::channel(EVENT_BUFFER);
        let (built, recorded) = tokio::join!(
            self.builder.build(&targets, events),
            self.record(run, queued, received)
        );
        recorded?;
        let built = built?;

        let mut statuses = Vec::with_capacity(queued.len());
        for (check, ok) in queued.iter().zip(built) {
            let status = if ok {
                CheckStatus::Passed
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
    async fn record(
        &self,
        run: RunId,
        queued: &[&Check],
        mut received: mpsc::Receiver<BuildEvent>,
    ) -> Result<(), StoreError> {
        let checks: HashMap<&str, CheckId> = queued
            .iter()
            .filter_map(|check| Some((check.derivation()?.path(), check.id())))
            .collect();
        let mut builds: HashMap<u64, String> = HashMap::new();
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
        Ok(())
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
