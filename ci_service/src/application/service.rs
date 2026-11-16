//! The [`Service`]: owns the ports and exposes every use case.
//! Why: one entry point for every caller — the HTTP adapter, and in the
//! monolith repohub or the CLI calling these methods directly — so rules are
//! enforced in one place.

mod live;
mod logs;
mod outbox;
mod pipeline;
mod retention;
#[cfg(test)]
mod tests;

use std::future::Future;

pub use logs::LogFollow;

use self::live::LiveLogs;
use super::ServiceError;
use super::ports::{
    Access, Builder, Clock, ConfigSource, Notifier, RunQuery, RunStore, StoreError,
};
use crate::domain::access::{Credential, Principal};
use crate::domain::check::Check;
use crate::domain::event::{EventKind, EventPayload, deliveries};
use crate::domain::log::NewLogLine;
use crate::domain::repo_config::{CachePolicy, Limits, RepoSettings, Retention};
use crate::domain::run::{NewRun, RepoName, Run, RunId, RunStatus};

/// Largest page of runs a caller can ask for.
const MAX_PAGE: u32 = 100;

/// The CI application: every use case, over swappable ports.
pub struct Service<S, C, B, R, N, A> {
    store: S,
    clock: C,
    builder: B,
    config: R,
    notifier: N,
    access: A,
    limits: Limits,
    retention: Retention,
    cache: Option<CachePolicy>,
    live: LiveLogs,
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
    /// Assemble the service: `notifier` delivers events to the repos'
    /// webhooks, `access` decides who reads which repo's runs. Default
    /// limits and log retention, no cache.
    #[must_use]
    pub fn new(store: S, clock: C, builder: B, config: R, notifier: N, access: A) -> Self {
        Self {
            store,
            clock,
            builder,
            config,
            notifier,
            access,
            limits: Limits::default(),
            retention: Retention::default(),
            cache: None,
            live: LiveLogs::default(),
        }
    }

    /// Cap what any repo config asks for at `limits`.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Keep run logs as `retention` says.
    #[must_use]
    pub fn with_retention(mut self, retention: Retention) -> Self {
        self.retention = retention;
        self
    }

    /// Push what each run builds to a cache chosen by `policy`.
    #[must_use]
    pub fn with_cache(mut self, policy: CachePolicy) -> Self {
        self.cache = Some(policy);
        self
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
        async move {
            // Why: read before the request moves into the store.
            let settings = self.config.repo_settings(run.repo()).await;
            let id = self.store.insert(run, self.clock.now()).await?;
            if let Ok(settings) = settings {
                self.announce(id, EventKind::RunQueued, RunStatus::Queued, &settings)
                    .await?;
            }
            Ok(id)
        }
    }

    /// Queue a lifecycle event for the webhooks that want it.
    async fn announce(
        &self,
        id: RunId,
        kind: EventKind,
        status: RunStatus,
        settings: &RepoSettings,
    ) -> Result<(), ServiceError> {
        let wanted = settings.webhooks().iter().any(|hook| hook.wants(kind));
        let Some(run) = (if wanted {
            self.store.run(id).await?
        } else {
            None
        }) else {
            return Ok(());
        };
        let event = [EventPayload::for_run(kind, &run, status)];
        let outbox = deliveries(&event, settings.webhooks());
        Ok(self
            .store
            .queue_deliveries(&outbox, self.clock.now())
            .await?)
    }

    /// Store lines of a run's log and hand them to its live readers.
    async fn append(&self, run: RunId, lines: &[NewLogLine<'_>]) -> Result<(), StoreError> {
        if lines.is_empty() {
            return Ok(());
        }
        let first = self.store.append_log(run, lines).await?;
        self.live.publish(run, first, lines);
        Ok(())
    }

    /// Fail unless `principal` may read `repo`.
    async fn authorize(&self, principal: &Principal, repo: &RepoName) -> Result<(), ServiceError> {
        if self.access.can_read(principal, repo).await? {
            Ok(())
        } else {
            Err(ServiceError::Forbidden)
        }
    }

    /// A run the holder of `credential` may read.
    async fn readable(&self, credential: Credential<'_>, id: RunId) -> Result<Run, ServiceError> {
        let principal = self.access.authenticate(credential).await?;
        let run = self.store.run(id).await?.ok_or(ServiceError::NotFound)?;
        self.authorize(&principal, run.request().repo()).await?;
        Ok(run)
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
            let now = self.clock.now();
            let keep_until = now + self.retention.seconds(None);
            let finished = self
                .store
                .finish_with_outbox(id, status, now, &[], Some(keep_until))
                .await?;
            self.live.close(id);
            if finished {
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
    /// [`ServiceError::Unauthenticated`] for an invalid credential,
    /// [`ServiceError::NotFound`] for an unknown run,
    /// [`ServiceError::Forbidden`] when the caller may not read its repo,
    /// [`ServiceError::Internal`] when a port fails.
    pub fn run<'a>(
        &'a self,
        credential: Credential<'a>,
        id: RunId,
    ) -> impl Future<Output = Result<Run, ServiceError>> + Send + 'a {
        self.readable(credential, id)
    }

    /// A page of a repo's runs, newest first. `limit` is clamped to 1..=100.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unauthenticated`] for an invalid credential,
    /// [`ServiceError::Forbidden`] when the caller may not read the repo,
    /// [`ServiceError::Internal`] when a port fails.
    pub fn runs<'a>(
        &'a self,
        credential: Credential<'a>,
        repo: &'a RepoName,
        before: Option<RunId>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<Run>, ServiceError>> + Send + 'a {
        async move {
            let principal = self.access.authenticate(credential).await?;
            self.authorize(&principal, repo).await?;
            let query = RunQuery::new(Some(repo), before, limit.clamp(1, MAX_PAGE));
            Ok(self.store.runs(query).await?)
        }
    }

    /// A run's checks, in evaluation order. Empty until it is evaluated.
    ///
    /// # Errors
    ///
    /// As [`Service::run`].
    pub fn checks<'a>(
        &'a self,
        credential: Credential<'a>,
        run: RunId,
    ) -> impl Future<Output = Result<Vec<Check>, ServiceError>> + Send + 'a {
        async move {
            self.readable(credential, run).await?;
            Ok(self.store.checks(run).await?)
        }
    }
}
