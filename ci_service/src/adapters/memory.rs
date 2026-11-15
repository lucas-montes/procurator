//! In-memory run store.
//! Why: fast tests of the use cases, and embedding without `SQLite`.

use std::future::{Future, ready};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::application::ports::{RunQuery, RunStore, StoreError};
use crate::domain::run::{NewRun, Run, RunId, RunStatus};

/// Runs held in a vector, in id order.
#[derive(Debug, Default)]
pub struct MemoryRunStore {
    runs: Mutex<Vec<Run>>,
}

impl MemoryRunStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Lock the runs, recovering them if a panicking thread poisoned the lock.
    fn runs(&self) -> MutexGuard<'_, Vec<Run>> {
        self.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Why: ids are positions + 1, so a lookup is an index.
fn index(id: RunId) -> Option<usize> {
    usize::try_from(id.get()).ok()?.checked_sub(1)
}

impl RunStore for MemoryRunStore {
    fn insert(
        &self,
        run: NewRun,
        now: i64,
    ) -> impl Future<Output = Result<RunId, StoreError>> + Send {
        let mut runs = self.runs();
        let id = i64::try_from(runs.len() + 1)
            .map(RunId::new)
            .map_err(StoreError::new);
        if let Ok(id) = id {
            runs.push(Run::queued(id, run, now));
        }
        ready(id)
    }

    fn claim_next(&self, now: i64) -> impl Future<Output = Result<Option<Run>, StoreError>> + Send {
        let mut runs = self.runs();
        let claimed = runs
            .iter_mut()
            .find(|run| run.status() == RunStatus::Queued)
            .map(|run| {
                run.start(now);
                run.clone()
            });
        ready(Ok(claimed))
    }

    fn requeue_running(&self) -> impl Future<Output = Result<u64, StoreError>> + Send {
        let mut runs = self.runs();
        let requeued = runs
            .iter_mut()
            .map(Run::requeue)
            .filter(|&changed| changed)
            .count();
        ready(Ok(u64::try_from(requeued).unwrap_or(u64::MAX)))
    }

    fn finish(
        &self,
        id: RunId,
        status: RunStatus,
        now: i64,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send {
        let mut runs = self.runs();
        let finished = index(id)
            .and_then(|i| runs.get_mut(i))
            .is_some_and(|run| run.finish(status, now));
        ready(Ok(finished))
    }

    fn run(&self, id: RunId) -> impl Future<Output = Result<Option<Run>, StoreError>> + Send {
        let runs = self.runs();
        ready(Ok(index(id).and_then(|i| runs.get(i)).cloned()))
    }

    fn runs(
        &self,
        query: RunQuery<'_>,
    ) -> impl Future<Output = Result<Vec<Run>, StoreError>> + Send {
        let runs = self.runs();
        let limit = usize::try_from(query.limit()).unwrap_or(usize::MAX);
        let page = runs
            .iter()
            .rev()
            .filter(|run| query.before().is_none_or(|before| run.id() < before))
            .filter(|run| query.repo().is_none_or(|repo| run.request().repo() == repo))
            .take(limit)
            .cloned()
            .collect();
        ready(Ok(page))
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
