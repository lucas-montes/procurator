//! Use-case tests against in-memory adapters: no HTTP and no `SQLite`.

use std::sync::atomic::{AtomicI64, Ordering};

use super::Service;
use crate::adapters::memory::MemoryRunStore;
use crate::adapters::scripted::{FixedConfig, ScriptedBuilder, derivation};
use crate::application::ServiceError;
use crate::application::ports::{BuildEvent, BuilderError, Clock, Evaluated};
use crate::domain::check::{CheckStatus, LOCAL_MACHINE};
use crate::domain::log::LogOrigin;
use crate::domain::repo_config::{
    CachePolicy, CheckSelection, Limits, RepoSettings, RunSettings, Timeouts,
};
use crate::domain::run::{CommitHash, GitRef, NewRun, RepoName, RunId, RunStatus};

/// A clock tests move by hand.
struct TestClock(AtomicI64);

impl Clock for &'static TestClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Generous service limits.
fn limits() -> Limits {
    Limits::new(5, 3600, 7200)
}

/// A service whose builder evaluates to nothing.
fn service() -> (
    Service<MemoryRunStore, &'static TestClock, ScriptedBuilder, FixedConfig>,
    &'static TestClock,
) {
    with_builder(ScriptedBuilder::new(&["x86_64-linux"], Ok(Vec::new())))
}

fn with_builder(
    builder: ScriptedBuilder,
) -> (
    Service<MemoryRunStore, &'static TestClock, ScriptedBuilder, FixedConfig>,
    &'static TestClock,
) {
    with_config(builder, FixedConfig::default(), limits())
}

fn with_config(
    builder: ScriptedBuilder,
    config: FixedConfig,
    limits: Limits,
) -> (
    Service<MemoryRunStore, &'static TestClock, ScriptedBuilder, FixedConfig>,
    &'static TestClock,
) {
    with_cache(builder, config, limits, None)
}

fn with_cache(
    builder: ScriptedBuilder,
    config: FixedConfig,
    limits: Limits,
    cache: Option<CachePolicy>,
) -> (
    Service<MemoryRunStore, &'static TestClock, ScriptedBuilder, FixedConfig>,
    &'static TestClock,
) {
    let clock: &'static TestClock = Box::leak(Box::new(TestClock(AtomicI64::new(100))));
    let store = MemoryRunStore::new();
    let service = Service::new(store, clock, builder, config, limits, cache);
    (service, clock)
}

fn policy() -> CachePolicy {
    CachePolicy::new(String::from("public"), String::from("private-"))
}

#[tokio::test]
async fn what_a_run_built_is_pushed_to_the_owners_cache() {
    let (service, _) = with_cache(
        mixed_builder(),
        FixedConfig::default(),
        limits(),
        Some(policy()),
    );
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());

    assert_eq!(
        service.builder.pushed(),
        [(
            String::from("private-lucas"),
            vec![String::from(DEP), String::from(FAIL), String::from(PASS)]
        )],
        "every derivation that started, dependencies included, once each"
    );
    let log = service.run_log(id).await.unwrap();
    assert_eq!(
        lines(&log).last(),
        Some(&"pushed 3 paths to cache private-lucas")
    );
}

#[tokio::test]
async fn the_default_branch_picks_the_cache_but_not_someone_elses() {
    for (requested, expected) in [("public", Some("public")), ("private-alice", None)] {
        let repo = RepoSettings::new(Some(String::from(requested)));
        let config = FixedConfig::default().with_repo(repo);
        let (service, _) = with_cache(mixed_builder(), config, limits(), Some(policy()));
        let id = service.enqueue(request("lucas/dummy")).await.unwrap();
        assert!(service.work_next().await.unwrap());

        let pushed: Vec<String> = service
            .builder
            .pushed()
            .into_iter()
            .map(|(cache, _)| cache)
            .collect();
        assert_eq!(
            pushed,
            expected.into_iter().map(String::from).collect::<Vec<_>>()
        );
        let log = service.run_log(id).await.unwrap();
        let last = lines(&log).last().copied().unwrap_or_default();
        if expected.is_none() {
            assert!(last.starts_with("warning: not pushed to a cache"), "{last}");
        }
    }
}

#[tokio::test]
async fn nothing_built_means_nothing_pushed() {
    let evaluation = vec![derivation("checks.x.a", "x86_64-linux", PASS, true)];
    let builder = ScriptedBuilder::new(&["x86_64-linux"], Ok(evaluation));
    let (service, _) = with_cache(builder, FixedConfig::default(), limits(), Some(policy()));
    service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());
    assert!(service.builder.pushed().is_empty());
}

#[tokio::test]
async fn failed_checks_are_rerun_and_flaky_ones_show_their_tries() {
    let evaluation = vec![
        derivation("checks.x.flaky", "x86_64-linux", PASS, false),
        derivation("checks.x.broken", "x86_64-linux", FAIL, false),
    ];
    let builder = ScriptedBuilder::new(&["x86_64-linux"], Ok(evaluation)).passing_from(PASS, 2);
    let settings = RunSettings::new(CheckSelection::All, 2, Timeouts::default());
    let (service, _) = with_config(builder, FixedConfig::settings(settings), limits());
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());

    assert_eq!(service.run(id).await.unwrap().status(), RunStatus::Failed);
    assert_eq!(
        service.builder.built(),
        [PASS, FAIL, PASS, FAIL, FAIL],
        "reruns build only what failed"
    );
    let checks = service.checks(id).await.unwrap();
    let summary: Vec<(CheckStatus, u32)> = checks
        .iter()
        .map(|check| (check.status(), check.tries()))
        .collect();
    assert_eq!(
        summary,
        [(CheckStatus::Passed, 2), (CheckStatus::Failed, 3)]
    );
    let broken = service.check_log(id, "checks.x.broken").await.unwrap();
    assert_eq!(
        lines(&broken),
        [
            "failed, building again: try 2 of 3",
            "failed, building again: try 3 of 3"
        ]
    );
}

#[tokio::test]
async fn reruns_and_timeouts_are_capped_and_passed_to_the_builder() {
    let evaluation = vec![derivation("checks.x.a", "x86_64-linux", FAIL, false)];
    let builder = ScriptedBuilder::new(&["x86_64-linux"], Ok(evaluation));
    let selection = CheckSelection::attrs(vec![String::from("checks.x.a")]).unwrap();
    let asked = RunSettings::new(
        selection.clone(),
        50,
        Timeouts::new(9000, 9000, 9000).unwrap(),
    );
    let (service, _) = with_config(
        builder,
        FixedConfig::settings(asked),
        Limits::new(1, 100, 200),
    );
    service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());

    let capped = Timeouts::new(100, 100, 200).unwrap();
    assert_eq!(
        service.builder.asked(),
        [
            (selection, None),
            (CheckSelection::All, Some(capped)),
            (CheckSelection::All, Some(capped)),
        ],
        "the selection is evaluated, then one try plus one capped rerun"
    );
}

#[tokio::test]
async fn timed_out_builds_are_not_rerun() {
    let evaluation = vec![derivation("checks.x.slow", "x86_64-linux", FAIL, false)];
    let events = vec![
        BuildEvent::Started {
            build: 9,
            drv: String::from(FAIL),
            machine: String::new(),
        },
        BuildEvent::TimedOut { build: 9 },
    ];
    let builder = ScriptedBuilder::new(&["x86_64-linux"], Ok(evaluation)).with_events(events);
    let settings = RunSettings::new(CheckSelection::All, 3, Timeouts::default());
    let (service, _) = with_config(builder, FixedConfig::settings(settings), limits());
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());

    assert_eq!(service.run(id).await.unwrap().status(), RunStatus::Failed);
    assert_eq!(service.builder.built(), [FAIL]);
    let checks = service.checks(id).await.unwrap();
    assert_eq!(checks[0].status(), CheckStatus::TimedOut);
    let log = service.check_log(id, "checks.x.slow").await.unwrap();
    assert_eq!(lines(&log), ["error: build timed out"]);
}

#[tokio::test(start_paused = true)]
async fn a_run_past_its_time_limit_times_out() {
    let evaluation = vec![
        derivation("checks.x.hangs", "x86_64-linux", PASS, false),
        derivation("checks.x.cached", "x86_64-linux", DEP, true),
    ];
    let builder = ScriptedBuilder::new(&["x86_64-linux"], Ok(evaluation)).hanging();
    let settings = RunSettings::new(CheckSelection::All, 0, Timeouts::new(10, 10, 30).unwrap());
    let (service, _) = with_config(builder, FixedConfig::settings(settings), limits());
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());

    assert_eq!(service.run(id).await.unwrap().status(), RunStatus::TimedOut);
    let statuses: Vec<CheckStatus> = service
        .checks(id)
        .await
        .unwrap()
        .iter()
        .map(crate::domain::check::Check::status)
        .collect();
    assert_eq!(statuses, [CheckStatus::TimedOut, CheckStatus::Cached]);
    let log = service.run_log(id).await.unwrap();
    assert_eq!(lines(&log), ["run timed out after 30 seconds"]);
}

#[tokio::test]
async fn an_invalid_repo_config_fails_the_run_without_evaluating() {
    let builder = ScriptedBuilder::new(&["x86_64-linux"], Ok(Vec::new()));
    let config = FixedConfig::invalid("procurator.nix: unknown field `rerun`");
    let (service, _) = with_config(builder, config, limits());
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());

    assert_eq!(service.run(id).await.unwrap().status(), RunStatus::Failed);
    assert!(service.builder.asked().is_empty(), "nothing evaluated");
    let log = service.run_log(id).await.unwrap();
    assert_eq!(lines(&log), ["procurator.nix: unknown field `rerun`"]);
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
