//! Events: what happened to a run or a check, as sent to subscribers.
//! Why: which events a finished run produces (including "recovered") is a
//! rule, and the payload carries everything a consumer needs so no adapter
//! ever calls back into the service.

use serde::{Deserialize, Serialize};

use super::check::{Check, CheckStatus};
use super::run::{Run, RunStatus};

/// Longest accepted secret name.
const MAX_SECRET_NAME: usize = 128;

/// The kinds of events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventKind {
    /// A push was queued.
    #[serde(rename = "run.queued")]
    RunQueued,
    /// A worker started the run.
    #[serde(rename = "run.started")]
    RunStarted,
    /// A check failed or timed out, after its last try.
    #[serde(rename = "check.failed")]
    CheckFailed,
    /// A check passed or was cached.
    #[serde(rename = "check.passed")]
    CheckPassed,
    /// No machine could build a check.
    #[serde(rename = "check.skipped")]
    CheckSkipped,
    /// Every check passed, was cached or skipped.
    #[serde(rename = "run.succeeded")]
    RunSucceeded,
    /// At least one check failed or timed out, or the run timed out.
    #[serde(rename = "run.failed")]
    RunFailed,
    /// The run succeeded on the default branch after a failed one.
    #[serde(rename = "run.recovered")]
    RunRecovered,
    /// An infrastructure fault, not the repository's.
    #[serde(rename = "run.errored")]
    RunErrored,
}

/// A webhook subscription from the repo config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Webhook {
    url_secret: String,
    events: Vec<EventKind>,
}

impl Webhook {
    /// A webhook whose URL is the secret named `url_secret`.
    ///
    /// # Errors
    ///
    /// When the name is empty, too long or not `[A-Za-z0-9._-]`, or no
    /// event is listed.
    pub fn new(url_secret: String, events: Vec<EventKind>) -> Result<Self, String> {
        let valid = !url_secret.is_empty()
            && url_secret.len() <= MAX_SECRET_NAME
            && url_secret
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if !valid {
            return Err(format!(
                "webhook urlSecret {url_secret:?} must be 1-{MAX_SECRET_NAME} of A-Z a-z 0-9 . _ -"
            ));
        }
        if events.is_empty() {
            return Err(format!("webhook {url_secret:?} lists no events"));
        }
        Ok(Self { url_secret, events })
    }

    /// The secret holding the URL.
    #[must_use]
    pub fn url_secret(&self) -> &str {
        &self.url_secret
    }

    /// Whether it wants `kind`.
    #[must_use]
    pub fn wants(&self, kind: EventKind) -> bool {
        self.events.contains(&kind)
    }
}

/// Where a delivery goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A webhook, by the secret holding its URL.
    Webhook(String),
}

/// One event as sent: self-contained JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPayload {
    kind: EventKind,
    run: i64,
    repo: String,
    commit: String,
    #[serde(rename = "ref")]
    git_ref: String,
    /// The run's status, or the check's for check events.
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    check: Option<String>,
    /// Output paths of every passed or cached check, for `run.succeeded`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    outputs: Vec<String>,
}

impl EventPayload {
    /// The kind.
    #[must_use]
    pub fn kind(&self) -> EventKind {
        self.kind
    }

    /// The run's id.
    #[must_use]
    pub fn run(&self) -> i64 {
        self.run
    }

    /// A run-level event.
    #[must_use]
    pub fn for_run(kind: EventKind, run: &Run, status: RunStatus) -> Self {
        let request = run.request();
        Self {
            kind,
            run: run.id().get(),
            repo: String::from(request.repo().as_str()),
            commit: String::from(request.commit().as_str()),
            git_ref: String::from(request.git_ref().as_str()),
            status: String::from(status.as_str()),
            check: None,
            outputs: Vec::new(),
        }
    }

    fn for_check(kind: EventKind, run: &Run, check: &Check) -> Self {
        let mut payload = Self::for_run(kind, run, RunStatus::Running);
        payload.status = String::from(check.status().as_str());
        payload.check = Some(String::from(check.attr()));
        payload
    }

    /// The events of a run that ended with `status` and these `checks`.
    /// `previous` is the last finished, non-errored run of the same ref,
    /// and `on_default_branch` whether that ref is the default branch.
    #[must_use]
    pub fn for_finished(
        run: &Run,
        status: RunStatus,
        checks: &[Check],
        previous: Option<RunStatus>,
        on_default_branch: bool,
    ) -> Vec<Self> {
        let mut events: Vec<Self> = checks
            .iter()
            .filter_map(|check| {
                let kind = match check.status() {
                    CheckStatus::Passed | CheckStatus::Cached => EventKind::CheckPassed,
                    CheckStatus::Failed | CheckStatus::TimedOut => EventKind::CheckFailed,
                    CheckStatus::Skipped => EventKind::CheckSkipped,
                    CheckStatus::Queued | CheckStatus::Building => return None,
                };
                Some(Self::for_check(kind, run, check))
            })
            .collect();
        match status {
            RunStatus::Passed => {
                let mut succeeded = Self::for_run(EventKind::RunSucceeded, run, status);
                succeeded.outputs = checks
                    .iter()
                    .filter(|check| {
                        matches!(check.status(), CheckStatus::Passed | CheckStatus::Cached)
                    })
                    .filter_map(Check::derivation)
                    .flat_map(|drv| drv.outputs().iter().cloned())
                    .collect();
                events.push(succeeded);
                let was_broken = matches!(previous, Some(RunStatus::Failed | RunStatus::TimedOut));
                if on_default_branch && was_broken {
                    events.push(Self::for_run(EventKind::RunRecovered, run, status));
                }
            }
            RunStatus::Failed | RunStatus::TimedOut => {
                events.push(Self::for_run(EventKind::RunFailed, run, status));
            }
            RunStatus::Errored => events.push(Self::for_run(EventKind::RunErrored, run, status)),
            RunStatus::Queued | RunStatus::Running => {}
        }
        events
    }
}

/// A delivery waiting in the outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    id: i64,
    target: Target,
    payload: EventPayload,
    attempts: u32,
}

impl Delivery {
    /// Reassemble a stored delivery; `attempts` made so far.
    #[must_use]
    pub fn new(id: i64, target: Target, payload: EventPayload, attempts: u32) -> Self {
        Self {
            id,
            target,
            payload,
            attempts,
        }
    }

    /// The outbox id.
    #[must_use]
    pub fn id(&self) -> i64 {
        self.id
    }

    /// Where it goes.
    #[must_use]
    pub fn target(&self) -> &Target {
        &self.target
    }

    /// What is sent.
    #[must_use]
    pub fn payload(&self) -> &EventPayload {
        &self.payload
    }

    /// Attempts made so far.
    #[must_use]
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Count one more attempt.
    pub fn attempted(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
    }
}

/// The deliveries `events` need for `webhooks`: one per webhook and event
/// it wants.
#[must_use]
pub fn deliveries(events: &[EventPayload], webhooks: &[Webhook]) -> Vec<(Target, EventPayload)> {
    let mut out = Vec::new();
    for webhook in webhooks {
        for event in events.iter().filter(|event| webhook.wants(event.kind)) {
            out.push((
                Target::Webhook(String::from(webhook.url_secret())),
                event.clone(),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{EventKind, EventPayload, Target, Webhook, deliveries};
    use crate::domain::check::{Check, CheckId, CheckStatus, Derivation, NewCheck};
    use crate::domain::run::{CommitHash, GitRef, NewRun, RepoName, Run, RunId, RunStatus};

    fn run() -> Run {
        let request = NewRun::new(
            RepoName::parse(String::from("lucas/demo")).unwrap(),
            CommitHash::parse("a".repeat(40)).unwrap(),
            GitRef::parse(String::from("refs/heads/main")).unwrap(),
            None,
        );
        Run::queued(RunId::new(7), request, 1)
    }

    fn check(attr: &str, status: CheckStatus) -> Check {
        let drv = Derivation::new(
            format!("/nix/store/{attr}.drv"),
            vec![format!("/nix/store/{attr}")],
        );
        let new = NewCheck::from_parts(String::from(attr), String::from("x"), Some(drv), status);
        Check::inserted(CheckId::new(1), RunId::new(7), new)
    }

    fn kinds(events: &[EventPayload]) -> Vec<EventKind> {
        events.iter().map(EventPayload::kind).collect()
    }

    #[test]
    fn a_passing_run_lists_its_checks_and_outputs() {
        let checks = [
            check("a", CheckStatus::Passed),
            check("b", CheckStatus::Cached),
            check("c", CheckStatus::Skipped),
        ];
        let events = EventPayload::for_finished(&run(), RunStatus::Passed, &checks, None, true);
        assert_eq!(
            kinds(&events),
            [
                EventKind::CheckPassed,
                EventKind::CheckPassed,
                EventKind::CheckSkipped,
                EventKind::RunSucceeded
            ]
        );
        let json = serde_json::to_value(&events[3]).unwrap();
        assert_eq!(json["kind"], "run.succeeded");
        assert_eq!(json["ref"], "refs/heads/main");
        assert_eq!(
            json["outputs"],
            serde_json::json!(["/nix/store/a", "/nix/store/b"])
        );
        let check_json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(check_json["check"], "a");
        assert_eq!(check_json["status"], "passed");
    }

    #[test]
    fn recovery_needs_the_default_branch_and_a_broken_previous_run() {
        let ends_with_recovered = |previous, on_default| {
            let events =
                EventPayload::for_finished(&run(), RunStatus::Passed, &[], previous, on_default);
            kinds(&events).last() == Some(&EventKind::RunRecovered)
        };
        assert!(ends_with_recovered(Some(RunStatus::Failed), true));
        assert!(ends_with_recovered(Some(RunStatus::TimedOut), true));
        assert!(!ends_with_recovered(Some(RunStatus::Failed), false));
        assert!(!ends_with_recovered(Some(RunStatus::Passed), true));
        assert!(!ends_with_recovered(None, true));
    }

    #[test]
    fn failures_and_errors() {
        let checks = [check("a", CheckStatus::TimedOut)];
        let failed = EventPayload::for_finished(&run(), RunStatus::TimedOut, &checks, None, true);
        assert_eq!(
            kinds(&failed),
            [EventKind::CheckFailed, EventKind::RunFailed]
        );
        let errored = EventPayload::for_finished(&run(), RunStatus::Errored, &[], None, true);
        assert_eq!(kinds(&errored), [EventKind::RunErrored]);
    }

    #[test]
    fn each_webhook_gets_the_events_it_wants() {
        let events = EventPayload::for_finished(
            &run(),
            RunStatus::Failed,
            &[check("a", CheckStatus::Failed)],
            None,
            true,
        );
        let hooks = [
            Webhook::new(String::from("slack"), vec![EventKind::RunFailed]).unwrap(),
            Webhook::new(
                String::from("all"),
                vec![EventKind::RunFailed, EventKind::CheckFailed],
            )
            .unwrap(),
        ];
        let out = deliveries(&events, &hooks);
        let summary: Vec<(Target, EventKind)> = out
            .into_iter()
            .map(|(target, event)| (target, event.kind()))
            .collect();
        assert_eq!(
            summary,
            [
                (Target::Webhook(String::from("slack")), EventKind::RunFailed),
                (Target::Webhook(String::from("all")), EventKind::CheckFailed),
                (Target::Webhook(String::from("all")), EventKind::RunFailed),
            ]
        );
    }

    #[test]
    fn webhook_names_are_checked() {
        assert!(Webhook::new(String::from("slack-alerts"), vec![EventKind::RunFailed]).is_ok());
        assert!(Webhook::new(String::from("https://x"), vec![EventKind::RunFailed]).is_err());
        assert!(Webhook::new(String::from("ok"), Vec::new()).is_err());
    }
}
