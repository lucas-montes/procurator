//! Use-case tests against in-memory adapters: no HTTP and no `SQLite`.

use std::sync::atomic::{AtomicI64, Ordering};

use super::Service;
use crate::adapters::memory::MemoryRunStore;
use crate::application::ServiceError;
use crate::application::ports::Clock;
use crate::domain::run::{CommitHash, GitRef, NewRun, RepoName, RunId, RunStatus};

/// A clock tests move by hand.
struct TestClock(AtomicI64);

impl Clock for &'static TestClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

fn service() -> (
    Service<MemoryRunStore, &'static TestClock>,
    &'static TestClock,
) {
    let clock: &'static TestClock = Box::leak(Box::new(TestClock(AtomicI64::new(100))));
    (Service::new(MemoryRunStore::new(), clock), clock)
}

fn request(repo: &str) -> NewRun {
    NewRun::new(
        RepoName::parse(String::from(repo)).unwrap(),
        CommitHash::parse("b".repeat(40)).unwrap(),
        GitRef::parse(String::from("refs/heads/main")).unwrap(),
        None,
    )
}

#[tokio::test]
async fn enqueued_run_is_claimed_then_finished() {
    let (service, clock) = service();
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();

    let queued = service.run(id).await.unwrap();
    assert_eq!(queued.status(), RunStatus::Queued);
    assert_eq!(queued.times().created_at(), 100);

    clock.0.store(105, Ordering::Relaxed);
    let claimed = service.claim_next().await.unwrap().unwrap();
    assert_eq!(claimed.id(), id);
    assert_eq!(claimed.status(), RunStatus::Running);
    assert_eq!(claimed.times().started_at(), Some(105));
    assert!(
        service.claim_next().await.unwrap().is_none(),
        "claimed once"
    );

    clock.0.store(110, Ordering::Relaxed);
    service.finish(id, RunStatus::Passed).await.unwrap();
    let done = service.run(id).await.unwrap();
    assert_eq!(done.status(), RunStatus::Passed);
    assert_eq!(done.times().finished_at(), Some(110));
}

#[tokio::test]
async fn claims_oldest_first() {
    let (service, _) = service();
    let first = service.enqueue(request("lucas/a")).await.unwrap();
    let second = service.enqueue(request("lucas/b")).await.unwrap();
    assert_eq!(service.claim_next().await.unwrap().unwrap().id(), first);
    assert_eq!(service.claim_next().await.unwrap().unwrap().id(), second);
}

#[tokio::test]
async fn finish_explains_refusals() {
    let (service, _) = service();
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();

    assert!(matches!(
        service.finish(id, RunStatus::Running).await,
        Err(ServiceError::NotFinal(RunStatus::Running))
    ));
    assert!(matches!(
        service.finish(id, RunStatus::Passed).await,
        Err(ServiceError::Conflict(RunStatus::Queued))
    ));
    assert!(matches!(
        service.finish(RunId::new(999), RunStatus::Passed).await,
        Err(ServiceError::NotFound)
    ));
}

#[tokio::test]
async fn recover_requeues_interrupted_runs() {
    let (service, _) = service();
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    service.claim_next().await.unwrap().unwrap();

    assert_eq!(service.recover().await.unwrap(), 1);
    assert_eq!(service.run(id).await.unwrap().status(), RunStatus::Queued);
    assert_eq!(service.claim_next().await.unwrap().unwrap().id(), id);
}

#[tokio::test]
async fn runs_filters_by_repo_and_pages() {
    let (service, _) = service();
    let a1 = service.enqueue(request("lucas/a")).await.unwrap();
    service.enqueue(request("lucas/b")).await.unwrap();
    let a2 = service.enqueue(request("lucas/a")).await.unwrap();

    let repo = RepoName::parse(String::from("lucas/a")).unwrap();
    let ids: Vec<RunId> = service
        .runs(Some(&repo), None, 10)
        .await
        .unwrap()
        .iter()
        .map(crate::domain::run::Run::id)
        .collect();
    assert_eq!(ids, [a2, a1]);

    let page = service.runs(Some(&repo), Some(a2), 10).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].id(), a1);

    assert_eq!(
        service.runs(None, None, 0).await.unwrap().len(),
        1,
        "limit clamps to 1"
    );
}
