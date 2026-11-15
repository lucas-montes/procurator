//! The [`Service`]: owns the ports and exposes every use case.
//! Why: one entry point for every caller — the HTTP adapter, and in the
//! monolith repohub or the CLI calling these methods directly — so rules are
//! enforced in one place.

mod pipeline;
#[cfg(test)]
mod tests;

use std::future::Future;

use super::ServiceError;
use super::ports::{Builder, Clock, RunQuery, RunStore};
use crate::domain::check::Check;
use crate::domain::log::{LogLine, LogOrigin};
use crate::domain::run::{NewRun, RepoName, Run, RunId, RunStatus};

/// Largest page of runs a caller can ask for.
const MAX_PAGE: u32 = 100;

/// The CI application: every use case, over swappable ports.
pub struct Service<S, C, B> {
    store: S,
    clock: C,
    builder: B,
}

impl<S, C, B> Service<S, C, B>
where
    S: RunStore,
    C: Clock,
    B: Builder,
{
    /// Assemble the service.
    #[must_use]
    pub fn new(store: S, clock: C, builder: B) -> Self {
        Self {
            store,
            clock,
            builder,
        }
    }

    /// Queue a run for a pushed commit.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Internal`] when the store fails.
    pub fn enqueue(
        &self,
        run: NewRun,
    ) -> impl Future<Output = Result<RunId, ServiceError>> + Send + '_ {
        async move { Ok(self.store.insert(run, self.clock.now()).await?) }
    }

    /// Put runs left `running` by a previous process back in the queue.
    /// Call once at startup, before any worker starts. Returns how many.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Internal`] when the store fails.
    pub fn recover(&self) -> impl Future<Output = Result<u64, ServiceError>> + Send + '_ {
        async move { Ok(self.store.requeue_running().await?) }
    }

    /// Claim the oldest queued run for a worker.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Internal`] when the store fails.
    pub fn claim_next(
        &self,
    ) -> impl Future<Output = Result<Option<Run>, ServiceError>> + Send + '_ {
        async move { Ok(self.store.claim_next(self.clock.now()).await?) }
    }

    /// End a running run with a final status.
    ///
    /// # Errors
    ///
    /// [`ServiceError::NotFinal`] for `queued` or `running`,
    /// [`ServiceError::NotFound`] for an unknown run,
    /// [`ServiceError::Conflict`] when the run is not running,
    /// [`ServiceError::Internal`] when the store fails.
    pub fn finish(
        &self,
        id: RunId,
        status: RunStatus,
    ) -> impl Future<Output = Result<(), ServiceError>> + Send + '_ {
        async move {
            if !status.is_final() {
                return Err(ServiceError::NotFinal(status));
            }
            if self.store.finish(id, status, self.clock.now()).await? {
                return Ok(());
            }
            // Why: the store only says "not changed"; tell callers why.
            match self.store.run(id).await? {
                None => Err(ServiceError::NotFound),
                Some(run) => Err(ServiceError::Conflict(run.status())),
            }
        }
    }

    /// One run.
    ///
    /// # Errors
    ///
    /// [`ServiceError::NotFound`] for an unknown run,
    /// [`ServiceError::Internal`] when the store fails.
    pub fn run(&self, id: RunId) -> impl Future<Output = Result<Run, ServiceError>> + Send + '_ {
        async move { self.store.run(id).await?.ok_or(ServiceError::NotFound) }
    }

    /// A page of runs, newest first. `limit` is clamped to 1..=100.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Internal`] when the store fails.
    pub fn runs<'a>(
        &'a self,
        repo: Option<&'a RepoName>,
        before: Option<RunId>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<Run>, ServiceError>> + Send + 'a {
        async move {
            let query = RunQuery::new(repo, before, limit.clamp(1, MAX_PAGE));
            Ok(self.store.runs(query).await?)
        }
    }

    /// A run's checks, in evaluation order. Empty until it is evaluated.
    ///
    /// # Errors
    ///
    /// [`ServiceError::NotFound`] for an unknown run,
    /// [`ServiceError::Internal`] when the store fails.
    pub fn checks(
        &self,
        run: RunId,
    ) -> impl Future<Output = Result<Vec<Check>, ServiceError>> + Send + '_ {
        async move {
            self.run(run).await?;
            Ok(self.store.checks(run).await?)
        }
    }

    /// Everything a run logged: evaluation, every build (dependencies
    /// included) and Nix's messages, in order.
    ///
    /// # Errors
    ///
    /// [`ServiceError::NotFound`] for an unknown run,
    /// [`ServiceError::Internal`] when the store fails.
    pub fn run_log(
        &self,
        run: RunId,
    ) -> impl Future<Output = Result<Vec<LogLine>, ServiceError>> + Send + '_ {
        async move {
            self.run(run).await?;
            Ok(self.store.log(run, &[]).await?)
        }
    }

    /// One check's log: its evaluation error or skip reason, and its own
    /// derivation's build output.
    ///
    /// # Errors
    ///
    /// [`ServiceError::NotFound`] for an unknown run,
    /// [`ServiceError::CheckNotFound`] for an attribute the run lacks,
    /// [`ServiceError::Internal`] when the store fails.
    pub fn check_log<'a>(
        &'a self,
        run: RunId,
        attr: &'a str,
    ) -> impl Future<Output = Result<Vec<LogLine>, ServiceError>> + Send + 'a {
        async move {
            let checks = self.checks(run).await?;
            let check = checks
                .iter()
                .find(|check| check.attr() == attr)
                .ok_or(ServiceError::CheckNotFound)?;
            let origin = LogOrigin::Check(check.attr());
            let lines = match check.derivation() {
                Some(drv) => {
                    let origins = [origin, LogOrigin::Derivation(drv.path())];
                    self.store.log(run, &origins).await?
                }
                None => self.store.log(run, &[origin]).await?,
            };
            Ok(lines)
        }
    }
}
