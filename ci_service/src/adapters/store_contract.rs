//! One behavioral contract every `RunStore` adapter must satisfy.
//! Why: the memory and `SQLite` stores must be interchangeable.

use crate::application::ports::{RunQuery, RunStore};
use crate::domain::run::{CommitHash, GitRef, NewRun, Pusher, RepoName, RunId, RunStatus};

fn request(repo: &str, pusher: Option<&str>) -> NewRun {
    NewRun::new(
        RepoName::parse(String::from(repo)).unwrap(),
        CommitHash::parse("c".repeat(40)).unwrap(),
        GitRef::parse(String::from("refs/heads/main")).unwrap(),
        pusher.map(|name| Pusher::parse(String::from(name)).unwrap()),
    )
}

/// Run every contract check against an empty store.
pub async fn check<S: RunStore>(store: &S) {
    round_trip(store).await;
    lifecycle(store).await;
    listing(store).await;
}

async fn round_trip<S: RunStore>(store: &S) {
    let original = request("lucas/round", Some("git"));
    let id = store.insert(original.clone(), 10).await.unwrap();
    let run = store.run(id).await.unwrap().unwrap();
    assert_eq!(run.id(), id);
    assert_eq!(run.request(), &original);
    assert_eq!(run.status(), RunStatus::Queued);
    assert_eq!(run.times().created_at(), 10);
    assert_eq!(run.times().started_at(), None);
    assert!(
        store
            .run(RunId::new(id.get() + 1000))
            .await
            .unwrap()
            .is_none()
    );

    // Leave it finished so later checks start with nothing queued.
    let claimed = store.claim_next(11).await.unwrap().unwrap();
    assert_eq!(claimed.id(), id);
    assert!(store.finish(id, RunStatus::Passed, 12).await.unwrap());
}

async fn lifecycle<S: RunStore>(store: &S) {
    let first = store.insert(request("lucas/life", None), 20).await.unwrap();
    let second = store.insert(request("lucas/life", None), 21).await.unwrap();

    assert!(
        !store.finish(first, RunStatus::Passed, 22).await.unwrap(),
        "queued cannot finish"
    );

    let claimed = store.claim_next(23).await.unwrap().unwrap();
    assert_eq!(claimed.id(), first, "oldest first");
    assert_eq!(claimed.status(), RunStatus::Running);
    assert_eq!(claimed.times().started_at(), Some(23));

    assert_eq!(store.requeue_running().await.unwrap(), 1);
    let requeued = store.run(first).await.unwrap().unwrap();
    assert_eq!(requeued.status(), RunStatus::Queued);
    assert_eq!(requeued.times().started_at(), None);

    assert_eq!(store.claim_next(24).await.unwrap().unwrap().id(), first);
    assert_eq!(store.claim_next(25).await.unwrap().unwrap().id(), second);
    assert!(store.claim_next(26).await.unwrap().is_none());

    assert!(store.finish(first, RunStatus::Failed, 27).await.unwrap());
    assert!(
        !store.finish(first, RunStatus::Passed, 28).await.unwrap(),
        "finishes once"
    );
    let done = store.run(first).await.unwrap().unwrap();
    assert_eq!(done.status(), RunStatus::Failed);
    assert_eq!(done.times().finished_at(), Some(27));

    assert!(store.finish(second, RunStatus::TimedOut, 29).await.unwrap());
    assert!(
        !store
            .finish(RunId::new(9999), RunStatus::Passed, 30)
            .await
            .unwrap()
    );
}

async fn listing<S: RunStore>(store: &S) {
    let a1 = store.insert(request("acme/a", None), 40).await.unwrap();
    let b = store.insert(request("acme/b", None), 41).await.unwrap();
    let a2 = store.insert(request("acme/a", None), 42).await.unwrap();
    let repo = RepoName::parse(String::from("acme/a")).unwrap();

    let ids = |runs: Vec<crate::domain::run::Run>| -> Vec<RunId> {
        runs.iter().map(crate::domain::run::Run::id).collect()
    };
    let only_a = store
        .runs(RunQuery::new(Some(&repo), None, 10))
        .await
        .unwrap();
    assert_eq!(ids(only_a), [a2, a1]);

    let paged = store
        .runs(RunQuery::new(Some(&repo), Some(a2), 10))
        .await
        .unwrap();
    assert_eq!(ids(paged), [a1]);

    let newest = store.runs(RunQuery::new(None, None, 2)).await.unwrap();
    assert_eq!(ids(newest), [a2, b]);
}
