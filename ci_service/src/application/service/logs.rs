//! Reading logs: stored, or followed while the run is going.
//! Why: a check's log is its own lines, its derivation's build and the
//! builds of the dependencies it needed in this run, each stored once.

use std::collections::HashSet;
use std::future::Future;

use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use super::Service;
use crate::application::ServiceError;
use crate::application::ports::{
    Access, Builder, Clock, ConfigSource, Notifier, RunStore, StoreError,
};
use crate::domain::access::Credential;
use crate::domain::check::{Check, Derivation};
use crate::domain::log::{LogLine, LogOrigin};
use crate::domain::run::RunId;

/// The lines of one check.
#[derive(Debug)]
struct CheckScope {
    check: Check,
    /// Dependencies linked to it so far.
    linked: HashSet<String>,
    /// Derivations seen in the stream that are not its own.
    /// Why: a link is stored before its derivation's first line, so one
    /// look at the store per new derivation is enough.
    unrelated: HashSet<String>,
}

/// A reader following a log as it grows: first what is stored, then each
/// line as it is appended, until the run ends.
/// Drive it with [`Service::follow_next`].
#[derive(Debug)]
pub struct LogFollow {
    run: RunId,
    /// `None`: the whole run.
    scope: Option<CheckScope>,
    backlog: std::vec::IntoIter<LogLine>,
    receiver: Option<broadcast::Receiver<LogLine>>,
    /// The last line handed out. Why: stored and live lines overlap.
    last: i64,
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
    /// Everything a run logged: evaluation, every build (dependencies
    /// included) and Nix's messages, in order.
    ///
    /// # Errors
    ///
    /// As [`Service::run`].
    pub fn run_log<'a>(
        &'a self,
        credential: Credential<'a>,
        run: RunId,
    ) -> impl Future<Output = Result<Vec<LogLine>, ServiceError>> + Send + 'a {
        async move {
            self.readable(credential, run).await?;
            Ok(self.store.log(run, &[]).await?)
        }
    }

    /// One check's log: its evaluation error or skip reason, its own
    /// derivation's build output, and the builds of the dependencies it
    /// needed in this run.
    ///
    /// # Errors
    ///
    /// As [`Service::run`], plus [`ServiceError::CheckNotFound`] for an
    /// attribute the run lacks.
    pub fn check_log<'a>(
        &'a self,
        credential: Credential<'a>,
        run: RunId,
        attr: &'a str,
    ) -> impl Future<Output = Result<Vec<LogLine>, ServiceError>> + Send + 'a {
        async move {
            self.readable(credential, run).await?;
            let mut scope = Some(self.check_scope(run, attr).await?);
            Ok(self.scoped_log(run, &mut scope).await?)
        }
    }

    /// Follow a run's whole log. See [`LogFollow`].
    ///
    /// # Errors
    ///
    /// As [`Service::run_log`].
    pub fn follow_run_log<'a>(
        &'a self,
        credential: Credential<'a>,
        run: RunId,
    ) -> impl Future<Output = Result<LogFollow, ServiceError>> + Send + 'a {
        async move {
            self.readable(credential, run).await?;
            self.follow(run, None).await
        }
    }

    /// Follow one check's log. See [`LogFollow`].
    /// Why: it ends with the run, not the check: a check's log is only
    /// complete once nothing else in the run can add a dependency to it.
    ///
    /// # Errors
    ///
    /// As [`Service::check_log`].
    pub fn follow_check_log<'a>(
        &'a self,
        credential: Credential<'a>,
        run: RunId,
        attr: &'a str,
    ) -> impl Future<Output = Result<LogFollow, ServiceError>> + Send + 'a {
        async move {
            self.readable(credential, run).await?;
            let scope = self.check_scope(run, attr).await?;
            self.follow(run, Some(scope)).await
        }
    }

    /// The next line of a followed log, waiting for it while the run is
    /// going; `None` once the run has ended and every line was handed out.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Internal`] when the store fails.
    pub fn follow_next<'a>(
        &'a self,
        follow: &'a mut LogFollow,
    ) -> impl Future<Output = Result<Option<LogLine>, ServiceError>> + Send + 'a {
        async move {
            loop {
                if let Some(line) = follow.backlog.next() {
                    follow.last = line.seq();
                    return Ok(Some(line));
                }
                let Some(receiver) = &mut follow.receiver else {
                    return Ok(None);
                };
                match receiver.recv().await {
                    Ok(line) => {
                        if line.seq() > follow.last
                            && self.in_scope(&mut follow.scope, &line).await?
                        {
                            follow.last = line.seq();
                            return Ok(Some(line));
                        }
                    }
                    Err(RecvError::Closed) => follow.receiver = None,
                    // Why: what this reader missed is in the store.
                    Err(RecvError::Lagged(_)) => {
                        let mut missed = self.scoped_log(follow.run, &mut follow.scope).await?;
                        missed.retain(|line| line.seq() > follow.last);
                        follow.backlog = missed.into_iter();
                    }
                }
            }
        }
    }

    /// Start following `run`. Subscribes before reading the store, so no
    /// line falls between the two.
    async fn follow(
        &self,
        run: RunId,
        mut scope: Option<CheckScope>,
    ) -> Result<LogFollow, ServiceError> {
        let receiver = self.live.subscribe(run);
        let over = self
            .store
            .run(run)
            .await?
            .is_none_or(|run| run.status().is_final());
        let receiver = if over {
            // Why: nothing will close a channel opened after the run ended.
            self.live.close(run);
            None
        } else {
            Some(receiver)
        };
        let backlog = self.scoped_log(run, &mut scope).await?;
        Ok(LogFollow {
            run,
            scope,
            backlog: backlog.into_iter(),
            receiver,
            last: 0,
        })
    }

    async fn check_scope(&self, run: RunId, attr: &str) -> Result<CheckScope, ServiceError> {
        let check = self
            .store
            .checks(run)
            .await?
            .into_iter()
            .find(|check| check.attr() == attr)
            .ok_or(ServiceError::CheckNotFound)?;
        Ok(CheckScope {
            check,
            linked: HashSet::new(),
            unrelated: HashSet::new(),
        })
    }

    /// The stored lines a scope wants, after refreshing its links.
    async fn scoped_log(
        &self,
        run: RunId,
        scope: &mut Option<CheckScope>,
    ) -> Result<Vec<LogLine>, StoreError> {
        let Some(CheckScope { check, linked, .. }) = scope else {
            return self.store.log(run, &[]).await;
        };
        linked.extend(self.store.linked_derivations(check.id()).await?);
        let mut origins = Vec::with_capacity(linked.len() + 2);
        origins.push(LogOrigin::Check(check.attr()));
        origins.extend(
            check
                .derivation()
                .map(|drv| LogOrigin::Derivation(drv.path())),
        );
        origins.extend(linked.iter().map(|drv| LogOrigin::Derivation(drv)));
        self.store.log(run, &origins).await
    }

    /// Whether a streamed line belongs to the scope.
    async fn in_scope(
        &self,
        scope: &mut Option<CheckScope>,
        line: &LogLine,
    ) -> Result<bool, StoreError> {
        let Some(CheckScope {
            check,
            linked,
            unrelated,
        }) = scope
        else {
            return Ok(true);
        };
        let drv = match line.origin() {
            LogOrigin::Run => return Ok(false),
            LogOrigin::Check(attr) => return Ok(attr == check.attr()),
            LogOrigin::Derivation(drv) => drv,
        };
        if check.derivation().map(Derivation::path) == Some(drv) || linked.contains(drv) {
            return Ok(true);
        }
        if unrelated.contains(drv) {
            return Ok(false);
        }
        linked.extend(self.store.linked_derivations(check.id()).await?);
        if linked.contains(drv) {
            return Ok(true);
        }
        unrelated.insert(String::from(drv));
        Ok(false)
    }
}
