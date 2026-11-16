//! Use-case tests against in-memory adapters: no HTTP and no `SQLite`.

use std::sync::atomic::{AtomicI64, Ordering};

use super::Service;
use crate::adapters::access::OpenAccess;
use crate::adapters::memory::MemoryRunStore;
use crate::adapters::scripted::{FixedConfig, RecordingNotifier, ScriptedBuilder, derivation};
use crate::application::ServiceError;
use crate::application::ports::{BuildEvent, BuilderError, Clock, Evaluated};
use crate::domain::access::Credential;
use crate::domain::check::{CheckStatus, LOCAL_MACHINE};
use crate::domain::deploy::{DEPLOYMENT_ATTR, DeploySettings};
use crate::domain::event::{EventKind, EventPayload, Target, Webhook};
use crate::domain::log::LogOrigin;
use crate::domain::repo_config::{
    CachePolicy, CheckSelection, Limits, RepoSettings, Retention, RunSettings, Timeouts,
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
    Service<
        MemoryRunStore,
        &'static TestClock,
        ScriptedBuilder,
        FixedConfig,
        RecordingNotifier,
        OpenAccess,
    >,
    &'static TestClock,
) {
    with_builder(ScriptedBuilder::new(&["x86_64-linux"], Ok(Vec::new())))
}

fn with_builder(
    builder: ScriptedBuilder,
) -> (
    Service<
        MemoryRunStore,
        &'static TestClock,
        ScriptedBuilder,
        FixedConfig,
        RecordingNotifier,
        OpenAccess,
    >,
    &'static TestClock,
) {
    with_config(builder, FixedConfig::default(), limits())
}

fn with_config(
    builder: ScriptedBuilder,
    config: FixedConfig,
    limits: Limits,
) -> (
    Service<
        MemoryRunStore,
        &'static TestClock,
        ScriptedBuilder,
        FixedConfig,
        RecordingNotifier,
        OpenAccess,
    >,
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
    Service<
        MemoryRunStore,
        &'static TestClock,
        ScriptedBuilder,
        FixedConfig,
        RecordingNotifier,
        OpenAccess,
    >,
    &'static TestClock,
) {
    assemble(builder, config, RecordingNotifier::default(), limits, cache)
}

fn assemble(
    builder: ScriptedBuilder,
    config: FixedConfig,
    notifier: RecordingNotifier,
    limits: Limits,
    cache: Option<CachePolicy>,
) -> (
    Service<
        MemoryRunStore,
        &'static TestClock,
        ScriptedBuilder,
        FixedConfig,
        RecordingNotifier,
        OpenAccess,
    >,
    &'static TestClock,
) {
    let clock: &'static TestClock = Box::leak(Box::new(TestClock(AtomicI64::new(100))));
    let store = MemoryRunStore::new();
    let mut service =
        Service::new(store, clock, builder, config, notifier, OpenAccess).with_limits(limits);
    if let Some(policy) = cache {
        service = service.with_cache(policy);
    }
    (service, clock)
}

/// No credential; [`OpenAccess`] lets it read everything.
const ANYONE: Credential<'static> = Credential::None;

/// Repo settings on `refs/heads/main` with one webhook per secret.
fn hooks(hooks: &[(&str, &[EventKind])]) -> RepoSettings {
    let webhooks = hooks
        .iter()
        .map(|(secret, events)| Webhook::new(String::from(*secret), events.to_vec()).unwrap())
        .collect();
    RepoSettings::new(None, webhooks).on_branch(String::from("refs/heads/main"))
}

fn kinds(delivered: &[(Target, EventPayload)]) -> Vec<(String, EventKind)> {
    delivered
        .iter()
        .map(|(target, event)| {
            let (Target::Webhook(name) | Target::Deploy(name)) = target;
            (name.clone(), event.kind())
        })
        .collect()
}

#[tokio::test]
async fn webhooks_get_exactly_the_events_they_want() {
    let repo = hooks(&[
        ("slack", &[EventKind::RunFailed]),
        (
            "all",
            &[
                EventKind::RunQueued,
                EventKind::RunStarted,
                EventKind::CheckFailed,
            ],
        ),
    ]);
    let config = FixedConfig::default().with_repo(repo);
    let (service, _) = assemble(
        mixed_builder(),
        config,
        RecordingNotifier::default(),
        limits(),
        None,
    );
    service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());
    assert_eq!(service.deliver_due().await.unwrap(), 5);
    assert_eq!(service.deliver_due().await.unwrap(), 0, "each is sent once");

    let all = |kind| (String::from("all"), kind);
    assert_eq!(
        kinds(&service.notifier.delivered()),
        [
            all(EventKind::RunQueued),
            all(EventKind::RunStarted),
            (String::from("slack"), EventKind::RunFailed),
            all(EventKind::CheckFailed),
            all(EventKind::CheckFailed),
        ]
    );
}

#[tokio::test]
async fn a_down_webhook_is_retried_then_given_up_without_blocking_others() {
    let repo = hooks(&[
        ("down", &[EventKind::RunFailed]),
        ("up", &[EventKind::RunFailed]),
    ]);
    let config = FixedConfig::default().with_repo(repo);
    let notifier = RecordingNotifier::failing_for(vec![Target::Webhook(String::from("down"))]);
    let (service, clock) = assemble(mixed_builder(), config, notifier, limits(), None);
    service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());

    assert_eq!(service.deliver_due().await.unwrap(), 2);
    assert_eq!(
        kinds(&service.notifier.delivered()),
        [(String::from("up"), EventKind::RunFailed)]
    );
    assert_eq!(service.deliver_due().await.unwrap(), 0, "the retry waits");
    for _ in 0..2 {
        clock.0.fetch_add(60, Ordering::Relaxed);
        assert_eq!(service.deliver_due().await.unwrap(), 1, "retried");
    }
    clock.0.fetch_add(3600, Ordering::Relaxed);
    assert_eq!(
        service.deliver_due().await.unwrap(),
        0,
        "given up after 3 tries"
    );
}

#[tokio::test]
async fn a_fixed_default_branch_announces_its_recovery() {
    let repo = hooks(&[("slack", &[EventKind::RunRecovered])]);
    let evaluation = |cached| Ok(vec![derivation("checks.x.a", "x86_64-linux", PASS, cached)]);
    let failing = ScriptedBuilder::new(&["x86_64-linux"], evaluation(false));
    let (service, _) = assemble(
        failing,
        FixedConfig::default().with_repo(repo),
        RecordingNotifier::default(),
        limits(),
        None,
    );
    service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());
    service.deliver_due().await.unwrap();
    assert!(
        service.notifier.delivered().is_empty(),
        "nothing recovered yet"
    );

    // The next run of the same branch finds the check cached and passes.
    *service.builder.evaluation_slot() = Some(evaluation(true));
    service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());
    service.deliver_due().await.unwrap();
    assert_eq!(
        kinds(&service.notifier.delivered()),
        [(String::from("slack"), EventKind::RunRecovered)]
    );
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
    let log = service.run_log(ANYONE, id).await.unwrap();
    assert_eq!(
        lines(&log).last(),
        Some(&"pushed 3 paths to cache private-lucas")
    );
}

#[tokio::test]
async fn the_default_branch_picks_the_cache_but_not_someone_elses() {
    for (requested, expected) in [("public", Some("public")), ("private-alice", None)] {
        let repo = RepoSettings::new(Some(String::from(requested)), Vec::new());
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
        let log = service.run_log(ANYONE, id).await.unwrap();
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

    assert_eq!(
        service.run(ANYONE, id).await.unwrap().status(),
        RunStatus::Failed
    );
    assert_eq!(
        service.builder.built(),
        [PASS, FAIL, PASS, FAIL, FAIL],
        "reruns build only what failed"
    );
    let checks = service.checks(ANYONE, id).await.unwrap();
    let summary: Vec<(CheckStatus, u32)> = checks
        .iter()
        .map(|check| (check.status(), check.tries()))
        .collect();
    assert_eq!(
        summary,
        [(CheckStatus::Passed, 2), (CheckStatus::Failed, 3)]
    );
    let broken = service
        .check_log(ANYONE, id, "checks.x.broken")
        .await
        .unwrap();
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

    assert_eq!(
        service.run(ANYONE, id).await.unwrap().status(),
        RunStatus::Failed
    );
    assert_eq!(service.builder.built(), [FAIL]);
    let checks = service.checks(ANYONE, id).await.unwrap();
    assert_eq!(checks[0].status(), CheckStatus::TimedOut);
    let log = service
        .check_log(ANYONE, id, "checks.x.slow")
        .await
        .unwrap();
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

    assert_eq!(
        service.run(ANYONE, id).await.unwrap().status(),
        RunStatus::TimedOut
    );
    let statuses: Vec<CheckStatus> = service
        .checks(ANYONE, id)
        .await
        .unwrap()
        .iter()
        .map(crate::domain::check::Check::status)
        .collect();
    assert_eq!(statuses, [CheckStatus::TimedOut, CheckStatus::Cached]);
    let log = service.run_log(ANYONE, id).await.unwrap();
    assert_eq!(lines(&log), ["run timed out after 30 seconds"]);
}

#[tokio::test]
async fn an_invalid_repo_config_fails_the_run_without_evaluating() {
    let builder = ScriptedBuilder::new(&["x86_64-linux"], Ok(Vec::new()));
    let config = FixedConfig::invalid("procurator.nix: unknown field `rerun`");
    let (service, _) = with_config(builder, config, limits());
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());

    assert_eq!(
        service.run(ANYONE, id).await.unwrap().status(),
        RunStatus::Failed
    );
    assert!(service.builder.asked().is_empty(), "nothing evaluated");
    let log = service.run_log(ANYONE, id).await.unwrap();
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
    assert_eq!(
        service.run(ANYONE, id).await.unwrap().status(),
        RunStatus::Failed
    );
    assert_eq!(
        service.builder.built(),
        [PASS, FAIL],
        "only checks neither cached, skipped nor failed to evaluate are built"
    );

    let checks = service.checks(ANYONE, id).await.unwrap();
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
        .check_log(ANYONE, id, "checks.x86_64-linux.pass")
        .await
        .unwrap();
    assert_eq!(lines(&pass), ["pass line"]);
    let fail = service
        .check_log(ANYONE, id, "checks.x86_64-linux.fail")
        .await
        .unwrap();
    assert_eq!(lines(&fail), ["boom"]);
    let bad = service
        .check_log(ANYONE, id, "checks.x86_64-linux.bad")
        .await
        .unwrap();
    assert_eq!(lines(&bad), ["error:", "  bad check"]);
    let mac = service
        .check_log(ANYONE, id, "checks.aarch64-darwin.mac")
        .await
        .unwrap();
    assert_eq!(lines(&mac), ["skipped: no builder for aarch64-darwin"]);
    assert!(matches!(
        service.check_log(ANYONE, id, "checks.nope").await,
        Err(ServiceError::CheckNotFound)
    ));

    let run_log = service.run_log(ANYONE, id).await.unwrap();
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
    assert_eq!(
        service.run(ANYONE, id).await.unwrap().status(),
        RunStatus::Passed
    );
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
        assert_eq!(service.run(ANYONE, id).await.unwrap().status(), status);
        assert!(service.checks(ANYONE, id).await.unwrap().is_empty());
        let log = service.run_log(ANYONE, id).await.unwrap();
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

    let queued = service.run(ANYONE, id).await.unwrap();
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
    let done = service.run(ANYONE, id).await.unwrap();
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
    assert_eq!(
        service.run(ANYONE, id).await.unwrap().status(),
        RunStatus::Queued
    );
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
        .runs(ANYONE, &repo, None, 10)
        .await
        .unwrap()
        .iter()
        .map(crate::domain::run::Run::id)
        .collect();
    assert_eq!(ids, [a2, a1]);

    let page = service.runs(ANYONE, &repo, Some(a2), 10).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].id(), a1);

    assert_eq!(
        service.runs(ANYONE, &repo, None, 0).await.unwrap().len(),
        1,
        "limit clamps to 1"
    );
}

#[tokio::test]
async fn a_dependency_built_in_the_run_shows_in_the_checks_that_need_it() {
    let builder = mixed_builder().needing(PASS, &[DEP, "/nix/store/x-never-built.drv"]);
    let (service, _) = with_builder(builder);
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());

    let pass = service
        .check_log(ANYONE, id, "checks.x86_64-linux.pass")
        .await
        .unwrap();
    assert_eq!(lines(&pass), ["dep line", "pass line"]);
    assert_eq!(pass[0].origin(), LogOrigin::Derivation(DEP));
    let fail = service
        .check_log(ANYONE, id, "checks.x86_64-linux.fail")
        .await
        .unwrap();
    assert_eq!(lines(&fail), ["boom"], "it did not need the dependency");
}

#[tokio::test]
async fn a_follower_gets_every_line_once_then_the_end() {
    let builder = mixed_builder().needing(PASS, &[DEP]);
    let (service, _) = with_builder(builder);
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    // Why: following a queued run waits for it to be worked.
    let mut whole = service.follow_run_log(ANYONE, id).await.unwrap();
    let early = service
        .follow_check_log(ANYONE, id, "checks.x86_64-linux.pass")
        .await;
    assert!(
        matches!(early, Err(ServiceError::CheckNotFound)),
        "no checks before evaluation"
    );

    let collect = async {
        let mut seen = Vec::new();
        while let Some(line) = service.follow_next(&mut whole).await.unwrap() {
            seen.push(line);
        }
        seen
    };
    let (worked, seen) = tokio::join!(service.work_next(), collect);
    assert!(worked.unwrap());
    let stored = service.run_log(ANYONE, id).await.unwrap();
    assert_eq!(seen, stored, "each stored line exactly once, in order");
    assert!(!seen.is_empty());

    // A run that is over replays its log and ends at once.
    let mut after = service
        .follow_check_log(ANYONE, id, "checks.x86_64-linux.pass")
        .await
        .unwrap();
    let mut replay = Vec::new();
    while let Some(line) = service.follow_next(&mut after).await.unwrap() {
        replay.push(line);
    }
    assert_eq!(lines(&replay), ["dep line", "pass line"]);
}

#[tokio::test]
async fn logs_are_deleted_after_the_repos_retention_but_runs_stay() {
    let config = FixedConfig::default().with_repo(RepoSettings::default().keeping_logs(Some(2)));
    let (service, clock) = with_config(mixed_builder(), config, limits());
    let service = service.with_retention(Retention::new(30, 90));
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());
    assert!(!service.run_log(ANYONE, id).await.unwrap().is_empty());

    let day = 86_400;
    clock.0.fetch_add(2 * day - 1, Ordering::Relaxed);
    assert_eq!(service.delete_expired_logs().await.unwrap(), 0);
    clock.0.fetch_add(1, Ordering::Relaxed);
    assert!(service.delete_expired_logs().await.unwrap() > 0);
    assert!(service.run_log(ANYONE, id).await.unwrap().is_empty());
    assert_eq!(
        service.run(ANYONE, id).await.unwrap().status(),
        RunStatus::Failed
    );
    assert_eq!(service.checks(ANYONE, id).await.unwrap().len(), 5);
}

const DEPLOYMENT_DRV: &str = "/nix/store/dep-procurator-deployment.drv";
const DEPLOYMENT_OUT: &str = "/nix/store/dep-procurator-deployment.drv-out";
const DEPLOYMENT_FILE: &str = r#"{"version":1,"vms":[{"id":"web","worker":"worker-1","kernel":"/nix/store/k/bzImage","initramfs":"/nix/store/i/initrd","cmdline":"console=ttyS0","disk":"/nix/store/d/nixos.img","cpus":2,"memoryMiB":1024}]}"#;

/// A builder whose run passes, deployment included, and holds `file`.
fn deploying_builder(file: &str) -> ScriptedBuilder {
    let evaluation = vec![
        derivation("checks.x86_64-linux.pass", "x86_64-linux", PASS, false),
        derivation(DEPLOYMENT_ATTR, "x86_64-linux", DEPLOYMENT_DRV, false),
    ];
    ScriptedBuilder::new(&["x86_64-linux"], Ok(evaluation))
        .passing(&[PASS, DEPLOYMENT_DRV])
        .with_file(DEPLOYMENT_OUT, file)
}

/// Repo settings deploying `branch` to `http://cp/deploy`, with a webhook.
fn deploying(branch: &str) -> FixedConfig {
    let deploy = DeploySettings::new(String::from("http://cp/deploy"), String::from(branch));
    let repo = hooks(&[("slack", &[EventKind::RunSucceeded])]).deploying(Some(deploy.unwrap()));
    FixedConfig::default().with_repo(repo)
}

#[tokio::test]
async fn a_passed_run_on_the_deploy_branch_deploys_what_it_built() {
    let (service, _) = with_config(
        deploying_builder(DEPLOYMENT_FILE),
        deploying("main"),
        limits(),
    );
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());
    assert_eq!(
        service.run(ANYONE, id).await.unwrap().status(),
        RunStatus::Passed
    );
    assert_eq!(service.builder.deployments_asked(), [true]);

    service.deliver_due().await.unwrap();
    let delivered = service.notifier.delivered();
    assert_eq!(
        kinds(&delivered),
        [
            (String::from("slack"), EventKind::RunSucceeded),
            (String::from("http://cp/deploy"), EventKind::RunSucceeded),
        ]
    );
    assert!(delivered[0].1.deployment().is_none());
    let deployment = delivered[1].1.deployment().unwrap();
    assert_eq!(deployment.path(), DEPLOYMENT_OUT);
    assert_eq!(deployment.vms()[0].id(), "web");
    let log = service.run_log(ANYONE, id).await.unwrap();
    assert!(
        log.iter()
            .any(|line| line.text().starts_with("deploying 1 vms from"))
    );
}

#[tokio::test]
async fn other_branches_do_not_deploy() {
    let (service, _) = with_config(
        deploying_builder(DEPLOYMENT_FILE),
        deploying("release"),
        limits(),
    );
    service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());
    assert_eq!(service.builder.deployments_asked(), [false]);
    service.deliver_due().await.unwrap();
    let delivered = service.notifier.delivered();
    assert!(
        delivered
            .iter()
            .all(|(target, _)| matches!(target, Target::Webhook(_)))
    );
}

#[tokio::test]
async fn a_broken_deployment_fails_the_run_and_deploys_nothing() {
    let bad = DEPLOYMENT_FILE.replace("\"cpus\":2", "\"cpus\":0");
    let (service, _) = with_config(deploying_builder(&bad), deploying("main"), limits());
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());
    assert_eq!(
        service.run(ANYONE, id).await.unwrap().status(),
        RunStatus::Failed
    );
    let log = service.run_log(ANYONE, id).await.unwrap();
    assert!(
        log.iter().any(|line| line
            .text()
            .starts_with("deploy: deployment file: vm web: cpus")),
        "{log:?}"
    );
    service.deliver_due().await.unwrap();
    assert!(service.notifier.delivered().is_empty());
}

#[tokio::test]
async fn a_failed_run_deploys_nothing() {
    let evaluation = vec![
        derivation("checks.x86_64-linux.fail", "x86_64-linux", FAIL, false),
        derivation(DEPLOYMENT_ATTR, "x86_64-linux", DEPLOYMENT_DRV, false),
    ];
    let builder = ScriptedBuilder::new(&["x86_64-linux"], Ok(evaluation))
        .passing(&[DEPLOYMENT_DRV])
        .with_file(DEPLOYMENT_OUT, DEPLOYMENT_FILE);
    let (service, _) = with_config(builder, deploying("main"), limits());
    let id = service.enqueue(request("lucas/dummy")).await.unwrap();
    assert!(service.work_next().await.unwrap());
    assert_eq!(
        service.run(ANYONE, id).await.unwrap().status(),
        RunStatus::Failed
    );
    service.deliver_due().await.unwrap();
    assert!(service.notifier.delivered().is_empty());
}
