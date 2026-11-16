//! One behavioral contract every `RunStore` adapter must satisfy.
//! Why: the memory and `SQLite` stores must be interchangeable.

use crate::application::ports::{NextAttempt, RunQuery, RunStore};
use crate::domain::check::{Check, CheckStatus, Derivation, NewCheck, Systems};
use crate::domain::deploy::{DeploySettings, Deployment};
use crate::domain::event::{EventKind, EventPayload, Target, deploy_delivery};
use crate::domain::log::{LogLine, LogOrigin, NewLogLine};
use crate::domain::run::{CommitHash, GitRef, NewRun, Pusher, RepoName, Run, RunId, RunStatus};

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
    // Why: expects nothing queued before it; listing leaves runs queued.
    checks_and_logs(store).await;
    retries_and_closing(store).await;
    outbox(store).await;
    retention(store).await;
    listing(store).await;
}

async fn retention<S: RunStore>(store: &S) {
    let line = [NewLogLine::new(LogOrigin::Run, "kept a while")];
    let mut runs = Vec::new();
    for keep in [Some(200), Some(300), None] {
        let id = store
            .insert(request("lucas/retention", None), 100)
            .await
            .unwrap();
        store.claim_next(101).await.unwrap().unwrap();
        store.append_log(id, &line).await.unwrap();
        let checks = store
            .insert_checks(
                id,
                vec![NewCheck::eval_failed(
                    String::from("checks.x.y"),
                    String::from("x"),
                )],
            )
            .await
            .unwrap();
        store
            .link_derivations(&[(checks[0].id(), "/nix/store/e-dep.drv")])
            .await
            .unwrap();
        assert!(
            store
                .finish_with_outbox(id, RunStatus::Passed, 102, &[], keep)
                .await
                .unwrap()
        );
        runs.push((id, checks[0].id()));
    }
    let lines = |run: RunId| async move { store.log(run, &[]).await.unwrap().len() };

    assert_eq!(store.delete_expired_logs(199).await.unwrap(), 0);
    assert_eq!(store.delete_expired_logs(250).await.unwrap(), 1);
    let (expired, expired_check) = runs[0];
    assert_eq!(lines(expired).await, 0);
    assert!(
        store
            .linked_derivations(expired_check)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store.checks(expired).await.unwrap().len(),
        1,
        "checks outlive their logs"
    );
    assert_eq!(lines(runs[1].0).await, 1);
    assert_eq!(store.delete_expired_logs(250).await.unwrap(), 0, "once");
    assert_eq!(store.delete_expired_logs(i64::MAX).await.unwrap(), 1);
    assert_eq!(lines(runs[2].0).await, 1, "kept forever without an expiry");
}

async fn outbox<S: RunStore>(store: &S) {
    let main = GitRef::parse(String::from("refs/heads/main")).unwrap();
    let repo = RepoName::parse(String::from("lucas/outbox")).unwrap();
    let first = store
        .insert(request("lucas/outbox", None), 70)
        .await
        .unwrap();
    assert_eq!(
        store.previous_status(&repo, &main, first).await.unwrap(),
        None
    );
    store.claim_next(71).await.unwrap().unwrap();
    let run = store.run(first).await.unwrap().unwrap();

    let failed = EventPayload::for_run(EventKind::RunFailed, &run, RunStatus::Failed);
    let hook = Target::Webhook(String::from("slack"));
    let outbox = [(hook.clone(), failed.clone())];
    assert!(
        store
            .finish_with_outbox(first, RunStatus::Failed, 72, &outbox, None)
            .await
            .unwrap()
    );
    assert!(
        !store
            .finish_with_outbox(first, RunStatus::Passed, 73, &outbox, None)
            .await
            .unwrap(),
        "a finished run queues nothing more"
    );

    let due = store.due_deliveries(72, 10).await.unwrap();
    assert_eq!(due.len(), 1, "exactly one delivery");
    assert_eq!(
        (due[0].target(), due[0].payload(), due[0].attempts()),
        (&hook, &failed, 0)
    );
    assert!(
        store.due_deliveries(71, 10).await.unwrap().is_empty(),
        "not due yet"
    );

    let id = due[0].id();
    store
        .record_attempt(id, NextAttempt::RetryAt(100), Some("connection refused"))
        .await
        .unwrap();
    assert!(store.due_deliveries(99, 10).await.unwrap().is_empty());
    let retried = store.due_deliveries(100, 10).await.unwrap();
    assert_eq!(retried[0].attempts(), 1);
    store
        .record_attempt(id, NextAttempt::Delivered, None)
        .await
        .unwrap();
    assert!(store.due_deliveries(i64::MAX, 10).await.unwrap().is_empty());

    store
        .queue_deliveries(&[(hook.clone(), failed.clone())], 80)
        .await
        .unwrap();
    let queued = store.due_deliveries(80, 10).await.unwrap();
    assert_eq!(queued.len(), 1);
    store
        .record_attempt(queued[0].id(), NextAttempt::GiveUp, Some("404"))
        .await
        .unwrap();
    assert!(store.due_deliveries(i64::MAX, 10).await.unwrap().is_empty());
    deploys(store, &run).await;

    // An errored run is no verdict: the failure before it still counts.
    let second = store
        .insert(request("lucas/outbox", None), 90)
        .await
        .unwrap();
    store.claim_next(91).await.unwrap().unwrap();
    assert!(store.finish(second, RunStatus::Errored, 92).await.unwrap());
    let third = store
        .insert(request("lucas/outbox", None), 93)
        .await
        .unwrap();
    assert_eq!(
        store.previous_status(&repo, &main, third).await.unwrap(),
        Some(RunStatus::Failed)
    );
    let other = GitRef::parse(String::from("refs/heads/dev")).unwrap();
    assert_eq!(
        store.previous_status(&repo, &other, third).await.unwrap(),
        None
    );
    store.claim_next(94).await.unwrap().unwrap();
    assert!(store.finish(third, RunStatus::Passed, 95).await.unwrap());
}

/// A deploy keeps its target and its deployment.
async fn deploys<S: RunStore>(store: &S, run: &Run) {
    let succeeded = EventPayload::for_run(EventKind::RunSucceeded, run, RunStatus::Passed);
    let settings =
        DeploySettings::new(String::from("http://cp/deploy"), String::from("main")).unwrap();
    let file = r#"{"version":1,"vms":[{"id":"web","worker":"w1","kernel":"/nix/store/k","initramfs":"/nix/store/i","cmdline":"x","disk":"/nix/store/d","cpus":1,"memoryMiB":512}]}"#;
    let deployment = Deployment::parse(String::from("/nix/store/p"), file).unwrap();
    let deploy = deploy_delivery(&[succeeded], &settings, deployment).unwrap();
    store
        .queue_deliveries(std::slice::from_ref(&deploy), 81)
        .await
        .unwrap();
    let queued = store.due_deliveries(81, 10).await.unwrap();
    assert_eq!(
        (queued[0].target(), queued[0].payload()),
        (&deploy.0, &deploy.1)
    );
    store
        .record_attempt(queued[0].id(), NextAttempt::Delivered, None)
        .await
        .unwrap();
}

async fn retries_and_closing<S: RunStore>(store: &S) {
    let run = store
        .insert(request("lucas/retry", None), 60)
        .await
        .unwrap();
    store.claim_next(61).await.unwrap().unwrap();
    let systems = Systems::new(vec![String::from("x")]);
    let new = (0..3)
        .map(|i| {
            NewCheck::evaluated(
                format!("checks.x.c{i}"),
                String::from("x"),
                Derivation::new(format!("/nix/store/{i}-c.drv"), Vec::new()),
                false,
                &systems,
            )
        })
        .collect();
    let checks = store.insert_checks(run, new).await.unwrap();
    assert!(checks.iter().all(|check| check.tries() == 1));
    let (flaky, open, building) = (checks[0].id(), checks[1].id(), checks[2].id());

    assert!(
        !store.retry_check(flaky).await.unwrap(),
        "only failed checks are retried"
    );
    store
        .update_check(flaky, CheckStatus::Building, Some("local"))
        .await
        .unwrap();
    store
        .update_check(flaky, CheckStatus::Failed, None)
        .await
        .unwrap();
    assert!(store.retry_check(flaky).await.unwrap());
    assert!(!store.retry_check(flaky).await.unwrap(), "now queued");
    store
        .update_check(building, CheckStatus::Building, None)
        .await
        .unwrap();

    let stored = store.checks(run).await.unwrap();
    assert_eq!(
        (stored[0].status(), stored[0].tries(), stored[0].machine()),
        (CheckStatus::Queued, 2, None)
    );

    store
        .update_check(flaky, CheckStatus::Passed, None)
        .await
        .unwrap();
    assert_eq!(
        store
            .close_open_checks(run, CheckStatus::TimedOut)
            .await
            .unwrap(),
        2
    );
    let closed = store.checks(run).await.unwrap();
    let statuses: Vec<CheckStatus> = closed.iter().map(Check::status).collect();
    assert_eq!(
        statuses,
        [
            CheckStatus::Passed,
            CheckStatus::TimedOut,
            CheckStatus::TimedOut
        ]
    );
    assert_eq!(closed[0].tries(), 2);
    assert_eq!(closed[1].id(), open);
    assert!(store.finish(run, RunStatus::TimedOut, 62).await.unwrap());
}

async fn checks_and_logs<S: RunStore>(store: &S) {
    let run = store
        .insert(request("lucas/checks", None), 50)
        .await
        .unwrap();
    let other = store
        .insert(request("lucas/other", None), 50)
        .await
        .unwrap();
    store.claim_next(51).await.unwrap().unwrap();

    let drv = Derivation::new(
        String::from("/nix/store/aaaa-unit.drv"),
        vec![
            String::from("/nix/store/bbbb-unit"),
            String::from("/nix/store/cccc-unit-doc"),
        ],
    );
    let systems = Systems::new(vec![String::from("x86_64-linux")]);
    let new = vec![
        NewCheck::evaluated(
            String::from("checks.x86_64-linux.unit"),
            String::from("x86_64-linux"),
            drv.clone(),
            false,
            &systems,
        ),
        NewCheck::eval_failed(
            String::from("checks.x86_64-linux.bad"),
            String::from("x86_64-linux"),
        ),
    ];
    let inserted = store.insert_checks(run, new).await.unwrap();
    assert_eq!(inserted.len(), 2);
    assert_ne!(inserted[0].id(), inserted[1].id());
    assert_eq!(
        store.checks(run).await.unwrap(),
        inserted,
        "round trip, in order"
    );
    assert!(store.checks(other).await.unwrap().is_empty());

    check_updates(store, run, &inserted, &drv).await;
    logs_and_requeue(store, run, other, &drv, &inserted).await;
}

async fn check_updates<S: RunStore>(store: &S, run: RunId, inserted: &[Check], drv: &Derivation) {
    let unit = inserted[0].id();
    assert!(
        store
            .update_check(unit, CheckStatus::Building, Some("ssh-ng://mac"))
            .await
            .unwrap()
    );
    assert!(
        store
            .update_check(unit, CheckStatus::Passed, None)
            .await
            .unwrap()
    );
    assert!(
        !store
            .update_check(unit, CheckStatus::Failed, None)
            .await
            .unwrap(),
        "a finished check does not move"
    );
    assert!(
        !store
            .update_check(inserted[1].id(), CheckStatus::Passed, None)
            .await
            .unwrap(),
        "an eval failure is already over"
    );
    let stored = store.checks(run).await.unwrap();
    assert_eq!(stored[0].status(), CheckStatus::Passed);
    assert_eq!(stored[0].machine(), Some("ssh-ng://mac"));
    assert_eq!(stored[0].derivation(), Some(drv));
    assert_eq!(stored[1].derivation(), None);
}

async fn logs_and_requeue<S: RunStore>(
    store: &S,
    run: RunId,
    other: RunId,
    drv: &Derivation,
    inserted: &[Check],
) {
    let lines = [
        NewLogLine::new(LogOrigin::Run, "evaluating"),
        NewLogLine::new(LogOrigin::Derivation("/nix/store/dddd-dep.drv"), "dep"),
        NewLogLine::new(LogOrigin::Derivation(drv.path()), "unit 1"),
        NewLogLine::new(LogOrigin::Check("checks.x86_64-linux.bad"), "error: x"),
    ];
    let first = store.append_log(run, &lines).await.unwrap();
    let next = store
        .append_log(
            run,
            &[NewLogLine::new(LogOrigin::Derivation(drv.path()), "unit 2")],
        )
        .await
        .unwrap();
    assert_eq!(next, first + 4, "a batch's lines are numbered one by one");
    store
        .append_log(other, &[NewLogLine::new(LogOrigin::Run, "elsewhere")])
        .await
        .unwrap();

    let text = |lines: Vec<LogLine>| -> Vec<String> {
        lines.iter().map(|line| String::from(line.text())).collect()
    };
    let all = store.log(run, &[]).await.unwrap();
    let seqs: Vec<i64> = all.iter().map(LogLine::seq).collect();
    assert_eq!(seqs, [first, first + 1, first + 2, first + 3, next]);
    assert_eq!(
        text(all),
        ["evaluating", "dep", "unit 1", "error: x", "unit 2"]
    );

    let unit = inserted[0].id();
    let dep = "/nix/store/dddd-dep.drv";
    store
        .link_derivations(&[(unit, dep), (unit, dep)])
        .await
        .unwrap();
    assert_eq!(
        store.linked_derivations(unit).await.unwrap(),
        [dep],
        "linking twice is linking once"
    );
    assert!(
        store
            .linked_derivations(inserted[1].id())
            .await
            .unwrap()
            .is_empty()
    );
    let unit_log = store
        .log(
            run,
            &[
                LogOrigin::Check("checks.x86_64-linux.unit"),
                LogOrigin::Derivation(drv.path()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(unit_log[0].origin(), LogOrigin::Derivation(drv.path()));
    assert_eq!(text(unit_log), ["unit 1", "unit 2"]);

    // An interrupted attempt leaves nothing behind once requeued.
    assert_eq!(store.requeue_running().await.unwrap(), 1);
    assert!(store.checks(run).await.unwrap().is_empty());
    assert!(store.log(run, &[]).await.unwrap().is_empty());
    assert!(store.linked_derivations(unit).await.unwrap().is_empty());
    assert_eq!(text(store.log(other, &[]).await.unwrap()), ["elsewhere"]);
    let again = store.claim_next(52).await.unwrap().unwrap();
    assert_eq!(again.id(), run);
    let reinserted = store
        .insert_checks(
            run,
            vec![NewCheck::eval_failed(
                String::from("checks.x.y"),
                String::from("x"),
            )],
        )
        .await
        .unwrap();
    assert!(
        reinserted[0].id() > inserted[1].id(),
        "ids are never reused"
    );
    assert!(store.finish(run, RunStatus::Failed, 53).await.unwrap());
    store.claim_next(54).await.unwrap().unwrap();
    assert!(store.finish(other, RunStatus::Passed, 55).await.unwrap());
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
