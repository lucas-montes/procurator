//! In-memory run store.
//! Why: fast tests of the use cases, and embedding without `SQLite`.

use std::future::{Future, ready};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::application::ports::{NextAttempt, RunQuery, RunStore, StoreError};
use crate::domain::check::{Check, CheckId, CheckStatus, NewCheck};
use crate::domain::event::{Delivery, EventPayload, Target};
use crate::domain::log::{LogLine, LogOrigin, NewLogLine};
use crate::domain::run::{GitRef, NewRun, RepoName, Run, RunId, RunStatus};

/// Everything stored, in id order.
#[derive(Debug, Default)]
struct State {
    runs: Vec<Run>,
    checks: Vec<Check>,
    /// Why: checks of requeued runs are dropped, so ids are not positions.
    last_check: i64,
    log: Vec<(RunId, LogLine)>,
    outbox: Vec<Queued>,
}

/// A delivery and when it is next due.
#[derive(Debug)]
struct Queued {
    delivery: Delivery,
    next_at: i64,
    pending: bool,
}

impl State {
    fn queue(&mut self, outbox: &[(Target, EventPayload)], now: i64) {
        for (target, payload) in outbox {
            let id = i64::try_from(self.outbox.len() + 1).unwrap_or(i64::MAX);
            self.outbox.push(Queued {
                delivery: Delivery::new(id, target.clone(), payload.clone(), 0),
                next_at: now,
                pending: true,
            });
        }
    }
}

/// Runs, checks and log lines held in vectors.
#[derive(Debug, Default)]
pub struct MemoryRunStore {
    state: Mutex<State>,
}

impl MemoryRunStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Lock the state, recovering it if a panicking thread poisoned the lock.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Why: ids are positions + 1, so a lookup is an index.
fn index(id: i64) -> Option<usize> {
    usize::try_from(id).ok()?.checked_sub(1)
}

/// The next id for a vector of `len` items.
fn next_id(len: usize) -> Result<i64, StoreError> {
    i64::try_from(len + 1).map_err(StoreError::new)
}

impl RunStore for MemoryRunStore {
    fn insert(
        &self,
        run: NewRun,
        now: i64,
    ) -> impl Future<Output = Result<RunId, StoreError>> + Send {
        let mut state = self.state();
        let id = next_id(state.runs.len()).map(RunId::new);
        if let Ok(id) = id {
            state.runs.push(Run::queued(id, run, now));
        }
        ready(id)
    }

    fn claim_next(&self, now: i64) -> impl Future<Output = Result<Option<Run>, StoreError>> + Send {
        let mut state = self.state();
        let claimed = state
            .runs
            .iter_mut()
            .find(|run| run.status() == RunStatus::Queued)
            .map(|run| {
                run.start(now);
                run.clone()
            });
        ready(Ok(claimed))
    }

    fn requeue_running(&self) -> impl Future<Output = Result<u64, StoreError>> + Send {
        let mut state = self.state();
        let requeued: Vec<RunId> = state
            .runs
            .iter_mut()
            .filter_map(|run| run.requeue().then_some(run.id()))
            .collect();
        state
            .checks
            .retain(|check| !requeued.contains(&check.run()));
        state.log.retain(|(run, _)| !requeued.contains(run));
        ready(Ok(u64::try_from(requeued.len()).unwrap_or(u64::MAX)))
    }

    fn finish_with_outbox(
        &self,
        id: RunId,
        status: RunStatus,
        now: i64,
        outbox: &[(Target, EventPayload)],
    ) -> impl Future<Output = Result<bool, StoreError>> + Send {
        let mut state = self.state();
        let finished = index(id.get())
            .and_then(|i| state.runs.get_mut(i))
            .is_some_and(|run| run.finish(status, now));
        if finished {
            state.queue(outbox, now);
        }
        ready(Ok(finished))
    }

    fn queue_deliveries(
        &self,
        outbox: &[(Target, EventPayload)],
        now: i64,
    ) -> impl Future<Output = Result<(), StoreError>> + Send {
        self.state().queue(outbox, now);
        ready(Ok(()))
    }

    fn due_deliveries(
        &self,
        now: i64,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<Delivery>, StoreError>> + Send {
        let state = self.state();
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let due = state
            .outbox
            .iter()
            .filter(|queued| queued.pending && queued.next_at <= now)
            .take(limit)
            .map(|queued| queued.delivery.clone())
            .collect();
        ready(Ok(due))
    }

    fn record_attempt(
        &self,
        id: i64,
        next: NextAttempt,
        _error: Option<&str>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send {
        let mut state = self.state();
        if let Some(queued) = state
            .outbox
            .iter_mut()
            .find(|queued| queued.delivery.id() == id)
        {
            queued.delivery.attempted();
            match next {
                NextAttempt::Delivered | NextAttempt::GiveUp => queued.pending = false,
                NextAttempt::RetryAt(at) => queued.next_at = at,
            }
        }
        ready(Ok(()))
    }

    fn previous_status<'a>(
        &'a self,
        repo: &'a RepoName,
        git_ref: &'a GitRef,
        before: RunId,
    ) -> impl Future<Output = Result<Option<RunStatus>, StoreError>> + Send + 'a {
        let state = self.state();
        let previous = state
            .runs
            .iter()
            .rev()
            .filter(|run| run.id() < before)
            .filter(|run| run.request().repo() == repo && run.request().git_ref() == git_ref)
            .map(Run::status)
            .find(|status| status.is_final() && *status != RunStatus::Errored);
        ready(Ok(previous))
    }

    fn run(&self, id: RunId) -> impl Future<Output = Result<Option<Run>, StoreError>> + Send {
        let state = self.state();
        ready(Ok(index(id.get()).and_then(|i| state.runs.get(i)).cloned()))
    }

    fn runs(
        &self,
        query: RunQuery<'_>,
    ) -> impl Future<Output = Result<Vec<Run>, StoreError>> + Send {
        let state = self.state();
        let limit = usize::try_from(query.limit()).unwrap_or(usize::MAX);
        let page = state
            .runs
            .iter()
            .rev()
            .filter(|run| query.before().is_none_or(|before| run.id() < before))
            .filter(|run| query.repo().is_none_or(|repo| run.request().repo() == repo))
            .take(limit)
            .cloned()
            .collect();
        ready(Ok(page))
    }

    fn insert_checks(
        &self,
        run: RunId,
        checks: Vec<NewCheck>,
    ) -> impl Future<Output = Result<Vec<Check>, StoreError>> + Send {
        let mut state = self.state();
        let mut inserted = Vec::with_capacity(checks.len());
        for check in checks {
            state.last_check += 1;
            let id = CheckId::new(state.last_check);
            inserted.push(Check::inserted(id, run, check));
        }
        state.checks.extend(inserted.iter().cloned());
        ready(Ok(inserted))
    }

    fn update_check(
        &self,
        id: CheckId,
        status: CheckStatus,
        machine: Option<&str>,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send {
        let mut state = self.state();
        let updated = state
            .checks
            .iter_mut()
            .find(|check| check.id() == id)
            .is_some_and(|check| check.advance(status, machine));
        ready(Ok(updated))
    }

    fn retry_check(&self, id: CheckId) -> impl Future<Output = Result<bool, StoreError>> + Send {
        let mut state = self.state();
        let retried = state
            .checks
            .iter_mut()
            .find(|check| check.id() == id)
            .is_some_and(Check::retry);
        ready(Ok(retried))
    }

    fn close_open_checks(
        &self,
        run: RunId,
        status: CheckStatus,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send {
        let mut state = self.state();
        let closed = state
            .checks
            .iter_mut()
            .filter(|check| check.run() == run && !check.status().is_final())
            .map(|check| check.advance(status, None))
            .filter(|&closed| closed)
            .count();
        ready(Ok(u64::try_from(closed).unwrap_or(u64::MAX)))
    }

    fn checks(&self, run: RunId) -> impl Future<Output = Result<Vec<Check>, StoreError>> + Send {
        let state = self.state();
        let checks = state
            .checks
            .iter()
            .filter(|check| check.run() == run)
            .cloned()
            .collect();
        ready(Ok(checks))
    }

    fn append_log(
        &self,
        run: RunId,
        lines: &[NewLogLine<'_>],
    ) -> impl Future<Output = Result<(), StoreError>> + Send {
        let mut state = self.state();
        state.log.extend(lines.iter().map(|line| {
            let stored = LogLine::new(String::from(line.origin().key()), String::from(line.text()));
            (run, stored)
        }));
        ready(Ok(()))
    }

    fn log(
        &self,
        run: RunId,
        origins: &[LogOrigin<'_>],
    ) -> impl Future<Output = Result<Vec<LogLine>, StoreError>> + Send {
        let state = self.state();
        let lines = state
            .log
            .iter()
            .filter(|(line_run, line)| {
                *line_run == run && (origins.is_empty() || origins.contains(&line.origin()))
            })
            .map(|(_, line)| line.clone())
            .collect();
        ready(Ok(lines))
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryRunStore;
    use crate::adapters::store_contract;

    #[tokio::test]
    async fn satisfies_the_store_contract() {
        store_contract::check(&MemoryRunStore::new()).await;
    }
}
