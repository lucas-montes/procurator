//! A check: one flake attribute of a run, evaluated and then cached, built,
//! skipped or failed.
//! Why: what a check's first status is, and what a run's result is given its
//! checks, are rules every builder and store must share.

use std::fmt;

use super::run::{RunId, RunStatus};

/// A stored value this version does not understand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidCheck;

impl fmt::Display for InvalidCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown check status")
    }
}

impl std::error::Error for InvalidCheck {}

/// A check's id, assigned by the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CheckId(i64);

impl CheckId {
    /// Wrap a stored id.
    #[must_use]
    pub fn new(id: i64) -> Self {
        Self(id)
    }

    /// The raw id.
    #[must_use]
    pub fn get(self) -> i64 {
        self.0
    }
}

/// Where a check is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    /// Waiting for the build.
    Queued,
    /// A machine is building it.
    Building,
    /// Built in this run.
    Passed,
    /// Its outputs already existed (in the cache or on the CI host).
    Cached,
    /// Evaluation or build failed.
    Failed,
    /// No machine can build its system.
    Skipped,
    /// Hit a time limit.
    TimedOut,
}

impl CheckStatus {
    /// The stored and serialized spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Building => "building",
            Self::Passed => "passed",
            Self::Cached => "cached",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::TimedOut => "timed_out",
        }
    }

    /// Read a stored spelling.
    ///
    /// # Errors
    ///
    /// [`InvalidCheck`] for an unknown spelling.
    pub fn parse(stored: &str) -> Result<Self, InvalidCheck> {
        match stored {
            "queued" => Ok(Self::Queued),
            "building" => Ok(Self::Building),
            "passed" => Ok(Self::Passed),
            "cached" => Ok(Self::Cached),
            "failed" => Ok(Self::Failed),
            "skipped" => Ok(Self::Skipped),
            "timed_out" => Ok(Self::TimedOut),
            _ => Err(InvalidCheck),
        }
    }

    /// Whether the check is over.
    #[must_use]
    pub fn is_final(self) -> bool {
        !matches!(self, Self::Queued | Self::Building)
    }

    /// The result of a run whose checks ended like this.
    /// Why: cached and skipped checks never fail a run; one failure or
    /// timeout does. No checks at all is a pass.
    #[must_use]
    pub fn run_outcome(statuses: impl IntoIterator<Item = Self>) -> RunStatus {
        let failed = statuses
            .into_iter()
            .any(|status| matches!(status, Self::Failed | Self::TimedOut));
        if failed {
            RunStatus::Failed
        } else {
            RunStatus::Passed
        }
    }
}

/// A store derivation and the output paths it produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derivation {
    path: String,
    outputs: Vec<String>,
}

impl Derivation {
    /// Describe a derivation.
    #[must_use]
    pub fn new(path: String, outputs: Vec<String>) -> Self {
        Self { path, outputs }
    }

    /// The `.drv` store path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The output store paths.
    #[must_use]
    pub fn outputs(&self) -> &[String] {
        &self.outputs
    }
}

/// The systems some machine (the CI host or a remote builder) can build.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Systems(Vec<String>);

impl Systems {
    /// Collect systems; duplicates are dropped.
    #[must_use]
    pub fn new(mut systems: Vec<String>) -> Self {
        systems.sort_unstable();
        systems.dedup();
        Self(systems)
    }

    /// Whether some machine builds `system`.
    #[must_use]
    pub fn contains(&self, system: &str) -> bool {
        self.0
            .binary_search_by(|known| known.as_str().cmp(system))
            .is_ok()
    }
}

/// One evaluated attribute, before the store gives it an id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCheck {
    attr: String,
    system: String,
    derivation: Option<Derivation>,
    status: CheckStatus,
}

impl NewCheck {
    /// An attribute that evaluated to a derivation.
    /// Why: this is where "cached, else buildable, else skipped" is decided.
    #[must_use]
    pub fn evaluated(
        attr: String,
        system: String,
        derivation: Derivation,
        cached: bool,
        buildable: &Systems,
    ) -> Self {
        let status = if cached {
            CheckStatus::Cached
        } else if buildable.contains(&system) {
            CheckStatus::Queued
        } else {
            CheckStatus::Skipped
        };
        Self {
            attr,
            system,
            derivation: Some(derivation),
            status,
        }
    }

    /// An attribute whose evaluation failed.
    #[must_use]
    pub fn eval_failed(attr: String, system: String) -> Self {
        Self {
            attr,
            system,
            derivation: None,
            status: CheckStatus::Failed,
        }
    }

    /// Reassemble stored parts.
    #[must_use]
    pub fn from_parts(
        attr: String,
        system: String,
        derivation: Option<Derivation>,
        status: CheckStatus,
    ) -> Self {
        Self {
            attr,
            system,
            derivation,
            status,
        }
    }

    /// The flake attribute, e.g. `checks.x86_64-linux.unit`.
    #[must_use]
    pub fn attr(&self) -> &str {
        &self.attr
    }

    /// The system it builds for.
    #[must_use]
    pub fn system(&self) -> &str {
        &self.system
    }

    /// What it evaluated to, unless evaluation failed.
    #[must_use]
    pub fn derivation(&self) -> Option<&Derivation> {
        self.derivation.as_ref()
    }

    /// Its status right after evaluation.
    #[must_use]
    pub fn status(&self) -> CheckStatus {
        self.status
    }
}

/// A stored check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    id: CheckId,
    run: RunId,
    evaluated: NewCheck,
    machine: Option<String>,
}

impl Check {
    /// Reassemble a stored check. `machine` is where it was built, when it
    /// was.
    #[must_use]
    pub fn from_parts(
        id: CheckId,
        run: RunId,
        evaluated: NewCheck,
        machine: Option<String>,
    ) -> Self {
        Self {
            id,
            run,
            evaluated,
            machine,
        }
    }

    /// The check's id.
    #[must_use]
    pub fn id(&self) -> CheckId {
        self.id
    }

    /// The run it belongs to.
    #[must_use]
    pub fn run(&self) -> RunId {
        self.run
    }

    /// The flake attribute.
    #[must_use]
    pub fn attr(&self) -> &str {
        self.evaluated.attr()
    }

    /// The system it builds for.
    #[must_use]
    pub fn system(&self) -> &str {
        self.evaluated.system()
    }

    /// What it evaluated to, unless evaluation failed.
    #[must_use]
    pub fn derivation(&self) -> Option<&Derivation> {
        self.evaluated.derivation()
    }

    /// Current status.
    #[must_use]
    pub fn status(&self) -> CheckStatus {
        self.evaluated.status
    }

    /// Where it was built: [`LOCAL_MACHINE`] or a remote builder's URI.
    #[must_use]
    pub fn machine(&self) -> Option<&str> {
        self.machine.as_deref()
    }

    /// Move to a new status, recording the machine when given.
    /// `false` when the check is already over.
    pub fn advance(&mut self, status: CheckStatus, machine: Option<&str>) -> bool {
        if self.evaluated.status.is_final() {
            return false;
        }
        self.evaluated.status = status;
        if let Some(machine) = machine {
            self.machine = Some(String::from(machine));
        }
        true
    }
}

/// The machine name recorded for builds on the CI host itself.
pub const LOCAL_MACHINE: &str = "local";

#[cfg(test)]
mod tests {
    use super::{Check, CheckId, CheckStatus, Derivation, NewCheck, Systems};
    use crate::domain::run::{RunId, RunStatus};

    fn drv() -> Derivation {
        Derivation::new(
            String::from("/nix/store/aaaa-x.drv"),
            vec![String::from("/nix/store/bbbb-x")],
        )
    }

    fn linux() -> Systems {
        Systems::new(vec![
            String::from("x86_64-linux"),
            String::from("aarch64-linux"),
            String::from("x86_64-linux"),
        ])
    }

    fn evaluated(system: &str, cached: bool) -> NewCheck {
        NewCheck::evaluated(
            String::from("checks.x.unit"),
            String::from(system),
            drv(),
            cached,
            &linux(),
        )
    }

    #[test]
    fn cached_wins_then_buildable_then_skipped() {
        assert_eq!(
            evaluated("x86_64-linux", true).status(),
            CheckStatus::Cached
        );
        assert_eq!(
            evaluated("aarch64-darwin", true).status(),
            CheckStatus::Cached,
            "a cached check needs no builder"
        );
        assert_eq!(
            evaluated("aarch64-linux", false).status(),
            CheckStatus::Queued
        );
        assert_eq!(
            evaluated("aarch64-darwin", false).status(),
            CheckStatus::Skipped
        );
        let failed = NewCheck::eval_failed(String::from("checks.x.bad"), String::from("x"));
        assert_eq!(failed.status(), CheckStatus::Failed);
        assert!(failed.derivation().is_none());
    }

    #[test]
    fn run_fails_only_on_failed_or_timed_out_checks() {
        use CheckStatus::{Cached, Failed, Passed, Skipped, TimedOut};
        assert_eq!(CheckStatus::run_outcome([]), RunStatus::Passed);
        assert_eq!(
            CheckStatus::run_outcome([Passed, Cached, Skipped]),
            RunStatus::Passed
        );
        assert_eq!(
            CheckStatus::run_outcome([Passed, Failed]),
            RunStatus::Failed
        );
        assert_eq!(CheckStatus::run_outcome([TimedOut]), RunStatus::Failed);
    }

    #[test]
    fn statuses_round_trip() {
        for status in [
            CheckStatus::Queued,
            CheckStatus::Building,
            CheckStatus::Passed,
            CheckStatus::Cached,
            CheckStatus::Failed,
            CheckStatus::Skipped,
            CheckStatus::TimedOut,
        ] {
            assert_eq!(CheckStatus::parse(status.as_str()), Ok(status));
        }
        assert!(CheckStatus::parse("done").is_err());
    }

    #[test]
    fn a_finished_check_does_not_move() {
        let mut check = Check::from_parts(
            CheckId::new(1),
            RunId::new(1),
            evaluated("x86_64-linux", false),
            None,
        );
        assert!(check.advance(CheckStatus::Building, Some("ssh-ng://b")));
        assert_eq!(check.machine(), Some("ssh-ng://b"));
        assert!(check.advance(CheckStatus::Passed, None));
        assert_eq!(check.machine(), Some("ssh-ng://b"), "kept when not given");
        assert!(!check.advance(CheckStatus::Failed, None));
        assert_eq!(check.status(), CheckStatus::Passed);
    }
}
