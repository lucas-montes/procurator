//! Use-case tests against in-memory adapters: no HTTP and no `SQLite`.

use std::sync::atomic::{AtomicI64, Ordering};

use super::Service;
use crate::adapters::memory::MemoryRunStore;
use crate::adapters::scripted::{ScriptedBuilder, derivation};
use crate::application::ServiceError;
use crate::application::ports::{BuildEvent, BuilderError, Clock, Evaluated};
use crate::domain::check::{CheckStatus, LOCAL_MACHINE};
use crate::domain::log::LogOrigin;
use crate::domain::run::{CommitHash, GitRef, NewRun, RepoName, RunId, RunStatus};

/// A clock tests move by hand.
struct TestClock(AtomicI64);

impl Clock for &'static TestClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// A service whose builder evaluates to nothing.
fn service() -> (
    Service<MemoryRunStore, &'static TestClock, ScriptedBuilder>,
    &'static TestClock,
) {
    with_builder(ScriptedBuilder::new(&["x86_64-linux"], Ok(Vec::new())))
}

fn with_builder(
    builder: ScriptedBuilder,
) -> (
    Service<MemoryRunStore, &'static TestClock, ScriptedBuilder>,
    &'static TestClock,
) {
    let clock: &'static TestClock = Box::leak(Box::new(TestClock(AtomicI64::new(100))));
    (Service::new(MemoryRunStore::new(), clock, builder), clock)
}

fn lines(log: &[crate::domain::log::LogLine]) -> Vec<&str> {
    log.iter().map(crate::domain::log::LogLine::text).collect()
}

const PASS: &str = "/nix/store/p-pass.drv";
const FAIL: &str = "/nix/store/f-fail.drv";
const DEP: &str = "/nix/store/d-dep.drv";

#[tokio::test]
async fn work_next_is_false_on_an_empty_queue() {
    let (service, _) = service();
    assert!(!service.work_next().await.unwrap());
}

/// A builder for one cached, one passing, one failing, one skipped and one
/// unevaluable check, plus a dependency build.
fn mixed_builder() -> ScriptedBuilder {
    let evaluation = vec![
        derivation(
            "checks.x86_64-linux.cached",
            "x86_64-linux",
            "/nix/store/c-c.drv",
            true,
        ),
        derivation("checks.x86_64-linux.pass", "x86_64-linux", PASS, false),
        derivation("checks.x86_64-linux.fail", "x86_64-linux", FAIL, false),
        derivation(
            "checks.aarch64-darwin.mac",
            "aarch64-darwin",
            "/nix/store/m-m.drv",
            false,
        ),
        Evaluated::Failed {
            attr: String::from("checks.x86_64-linux.bad"),
            system: String::from("x86_64-linux"),
            error: String::from("error:\n  bad check"),
        },
    ];
    let events = vec![
        BuildEvent::Started {
            build: 1,
            drv: String::from(DEP),
            machine: String::new(),
        },
        BuildEvent::Started {
            build: 2,
            drv: String::from(PASS),
            machine: String::from("ssh-ng://builder"),
        },
        BuildEvent::Line {
            build: 1,
            text: String::from("dep line"),
        },
        BuildEvent::Started {
            build: 3,
            drv: String::from(FAIL),
            machine: String::new(),
        },
        BuildEvent::Line {
            build: 2,
            text: String::from("pass line"),
        },
        BuildEvent::Line {
            build: 3,
            text: String::from("boom"),
        },
        BuildEvent::Message(String::from("error: Cannot build fail\n  exit 3")),
    ];
    ScriptedBuilder::new(&["x86_64-linux"], Ok(evaluation))
        .with_events(events)
        .passing(&[PASS])
}

#[tokio::test]
async fn pipeline_caches_skips_builds_and_logs_per_derivation() {
    let (service, _) = with_builder(mixed_builder());
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();

    assert!(service.work_next().await.unwrap());
    assert_eq!(service.run(id).await.unwrap().status(), RunStatus::Failed);
    assert_eq!(
        service.builder.built(),
        [PASS, FAIL],
        "only checks neither cached, skipped nor failed to evaluate are built"
    );

    let checks = service.checks(id).await.unwrap();
    let summary: Vec<(&str, CheckStatus, Option<&str>)> = checks
        .iter()
        .map(|check| (check.attr(), check.status(), check.machine()))
        .collect();
    assert_eq!(
        summary,
        [
            ("checks.x86_64-linux.cached", CheckStatus::Cached, None),
            (
                "checks.x86_64-linux.pass",
                CheckStatus::Passed,
                Some("ssh-ng://builder")
            ),
            (
                "checks.x86_64-linux.fail",
                CheckStatus::Failed,
                Some(LOCAL_MACHINE)
            ),
            ("checks.aarch64-darwin.mac", CheckStatus::Skipped, None),
            ("checks.x86_64-linux.bad", CheckStatus::Failed, None),
        ]
    );

    let pass = service
        .check_log(id, "checks.x86_64-linux.pass")
        .await
        .unwrap();
    assert_eq!(lines(&pass), ["pass line"]);
    let fail = service
        .check_log(id, "checks.x86_64-linux.fail")
        .await
        .unwrap();
    assert_eq!(lines(&fail), ["boom"]);
    let bad = service
        .check_log(id, "checks.x86_64-linux.bad")
        .await
        .unwrap();
    assert_eq!(lines(&bad), ["error:", "  bad check"]);
    let mac = service
        .check_log(id, "checks.aarch64-darwin.mac")
        .await
        .unwrap();
    assert_eq!(lines(&mac), ["skipped: no builder for aarch64-darwin"]);
    assert!(matches!(
        service.check_log(id, "checks.nope").await,
        Err(ServiceError::CheckNotFound)
    ));

    let run_log = service.run_log(id).await.unwrap();
    let dep = run_log
        .iter()
        .find(|line| line.text() == "dep line")
        .unwrap();
    assert_eq!(
        dep.origin(),
        LogOrigin::Derivation(DEP),
        "dependency lines stay at run level"
    );
    assert!(lines(&run_log).ends_with(&["error: Cannot build fail", "  exit 3"]));
}

#[tokio::test]
async fn all_cached_passes_without_building() {
    let evaluation = vec![derivation("checks.x.a", "x86_64-linux", PASS, true)];
    let (service, _) = with_builder(ScriptedBuilder::new(&[], Ok(evaluation)));
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());
    assert_eq!(service.run(id).await.unwrap().status(), RunStatus::Passed);
    assert!(service.builder.built().is_empty());
}

#[tokio::test]
async fn a_broken_flake_fails_and_a_broken_builder_errors() {
    for (error, status) in [
        (
            BuilderError::Flake(String::from("syntax error")),
            RunStatus::Failed,
        ),
        (
            BuilderError::Unavailable(String::from("no nix")),
            RunStatus::Errored,
        ),
    ] {
        let (service, _) = with_builder(ScriptedBuilder::new(&[], Err(error)));
        let id = service.enqueue(request("lucas/dummy")).await.unwrap();
        assert!(service.work_next().await.unwrap());
        assert_eq!(service.run(id).await.unwrap().status(), status);
        assert!(service.checks(id).await.unwrap().is_empty());
        let log = service.run_log(id).await.unwrap();
        assert_eq!(log.len(), 1, "the reason is logged");
    }
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
