//! What a repository's `procurator.nix` asks of a run, and the service's
//! caps on it.
//! Why: the commit being checked chooses its checks, reruns and timeouts,
//! but never beyond what the service allows.

use std::fmt;

use super::deploy::DeploySettings;
use super::event::Webhook;

/// Default seconds a derivation may build.
const DEFAULT_CHECK_TIMEOUT: u64 = 3600;
/// Default seconds a derivation may go without output.
const DEFAULT_SILENT_TIMEOUT: u64 = 600;
/// Default seconds a whole run may take.
const DEFAULT_RUN_TIMEOUT: u64 = 7200;
/// Most attributes an explicit check list may name.
const MAX_ATTRS: usize = 1000;
/// Days run logs are kept when neither the repo nor the service says.
const DEFAULT_RETENTION_DAYS: u32 = 30;
/// Most days a repo may keep run logs when the service does not say.
const MAX_RETENTION_DAYS: u32 = 365;
const SECONDS_PER_DAY: i64 = 86_400;

/// Why a repo config was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidConfig(String);

impl InvalidConfig {
    /// Explain a rejection.
    #[must_use]
    pub fn new(reason: String) -> Self {
        Self(reason)
    }
}

impl fmt::Display for InvalidConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "procurator.nix: {}", self.0)
    }
}

impl std::error::Error for InvalidConfig {}

/// Which flake attributes a run checks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CheckSelection {
    /// Every `checks.<system>.<name>`.
    #[default]
    All,
    /// Exactly these attribute paths, e.g. `packages.x86_64-linux.default`.
    Attrs(Vec<String>),
}

impl CheckSelection {
    /// An explicit list.
    ///
    /// # Errors
    ///
    /// When it is empty, longer than 1000, or an entry is not a dotted path
    /// of `[A-Za-z0-9_'+-]` segments.
    /// Why: entries are spliced into a Nix expression as quoted names, so
    /// nothing that could end a string may get through.
    pub fn attrs(attrs: Vec<String>) -> Result<Self, InvalidConfig> {
        if attrs.is_empty() || attrs.len() > MAX_ATTRS {
            return Err(InvalidConfig::new(format!(
                "ci.checks must list 1 to {MAX_ATTRS} attributes"
            )));
        }
        if let Some(bad) = attrs.iter().find(|attr| !valid_attr(attr)) {
            return Err(InvalidConfig::new(format!(
                "ci.checks entry {bad:?} is not an attribute path like checks.x86_64-linux.unit"
            )));
        }
        Ok(Self::Attrs(attrs))
    }
}

fn valid_attr(attr: &str) -> bool {
    attr.split('.').all(|segment| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'\'' | b'+' | b'-'))
    })
}

/// Time limits, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    check: u64,
    silent: u64,
    run: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            check: DEFAULT_CHECK_TIMEOUT,
            silent: DEFAULT_SILENT_TIMEOUT,
            run: DEFAULT_RUN_TIMEOUT,
        }
    }
}

impl Timeouts {
    /// Limits per derivation (`check`, `silent`) and per run.
    ///
    /// # Errors
    ///
    /// When any is zero.
    pub fn new(check: u64, silent: u64, run: u64) -> Result<Self, InvalidConfig> {
        if check == 0 || silent == 0 || run == 0 {
            return Err(InvalidConfig::new(String::from(
                "ci.timeouts must be at least 1 second",
            )));
        }
        Ok(Self { check, silent, run })
    }

    /// Seconds one derivation may build.
    #[must_use]
    pub fn check(self) -> u64 {
        self.check
    }

    /// Seconds one derivation may go without output.
    #[must_use]
    pub fn silent(self) -> u64 {
        self.silent
    }

    /// Seconds the whole run may take.
    #[must_use]
    pub fn run(self) -> u64 {
        self.run
    }
}

/// The service's ceilings on what a repo may ask for: the most reruns, and
/// the longest check and run timeouts, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    reruns: u32,
    check_timeout: u64,
    run_timeout: u64,
}

/// Three reruns, four hours per check, six per run.
impl Default for Limits {
    fn default() -> Self {
        Self::new(3, 4 * 3600, 6 * 3600)
    }
}

impl Limits {
    /// Describe the ceilings.
    #[must_use]
    pub fn new(reruns: u32, check_timeout: u64, run_timeout: u64) -> Self {
        Self {
            reruns,
            check_timeout,
            run_timeout,
        }
    }
}

/// The per-run part of a repo config, read from the commit being checked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunSettings {
    checks: CheckSelection,
    reruns: u32,
    timeouts: Timeouts,
}

impl RunSettings {
    /// Describe what the repo asks for.
    #[must_use]
    pub fn new(checks: CheckSelection, reruns: u32, timeouts: Timeouts) -> Self {
        Self {
            checks,
            reruns,
            timeouts,
        }
    }

    /// The same settings, held under the service's limits.
    /// Why: a pushed commit must not be able to occupy a worker forever.
    #[must_use]
    pub fn capped(self, limits: Limits) -> Self {
        let timeouts = self.timeouts;
        let check = timeouts.check.min(limits.check_timeout).max(1);
        Self {
            checks: self.checks,
            reruns: self.reruns.min(limits.reruns),
            timeouts: Timeouts {
                check,
                silent: timeouts.silent.min(check),
                run: timeouts.run.min(limits.run_timeout).max(1),
            },
        }
    }

    /// Which attributes to check.
    #[must_use]
    pub fn checks(&self) -> &CheckSelection {
        &self.checks
    }

    /// How many times a failed check is built again.
    #[must_use]
    pub fn reruns(&self) -> u32 {
        self.reruns
    }

    /// Time limits.
    #[must_use]
    pub fn timeouts(&self) -> Timeouts {
        self.timeouts
    }
}

/// The repo-level part of a repo config, read from the default branch head.
/// Why: a feature branch must not reroute where its outputs go.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoSettings {
    cache: Option<String>,
    webhooks: Vec<Webhook>,
    log_retention_days: Option<u32>,
    default_ref: Option<String>,
    deploy: Option<DeploySettings>,
}

impl RepoSettings {
    /// Describe the repo-level settings. `cache` is `ci.cache`.
    #[must_use]
    pub fn new(cache: Option<String>, webhooks: Vec<Webhook>) -> Self {
        Self {
            cache,
            webhooks,
            log_retention_days: None,
            default_ref: None,
            deploy: None,
        }
    }

    /// The same settings, deploying as `deploy` says (`deploy`, when its
    /// `url` is set).
    #[must_use]
    pub fn deploying(mut self, deploy: Option<DeploySettings>) -> Self {
        self.deploy = deploy;
        self
    }

    /// Where and for which branch runs deploy, if they do.
    #[must_use]
    pub fn deploy(&self) -> Option<&DeploySettings> {
        self.deploy.as_ref()
    }

    /// The same settings, asking to keep run logs `days`
    /// (`ci.logRetentionDays`).
    #[must_use]
    pub fn keeping_logs(mut self, days: Option<u32>) -> Self {
        self.log_retention_days = days;
        self
    }

    /// Days the repo asked to keep run logs, if it did.
    #[must_use]
    pub fn log_retention_days(&self) -> Option<u32> {
        self.log_retention_days
    }

    /// The same settings, read from the head of `default_ref`
    /// (e.g. `refs/heads/main`).
    #[must_use]
    pub fn on_branch(mut self, default_ref: String) -> Self {
        self.default_ref = Some(default_ref);
        self
    }

    /// The cache the repo asked for, if any.
    #[must_use]
    pub fn cache(&self) -> Option<&str> {
        self.cache.as_deref()
    }

    /// Where events go.
    #[must_use]
    pub fn webhooks(&self) -> &[Webhook] {
        &self.webhooks
    }

    /// The default branch's ref, when the repo has one.
    #[must_use]
    pub fn default_ref(&self) -> Option<&str> {
        self.default_ref.as_deref()
    }
}

/// A whole parsed `procurator.nix`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoConfig {
    run: RunSettings,
    repo: RepoSettings,
}

impl RepoConfig {
    /// Combine both parts.
    #[must_use]
    pub fn new(run: RunSettings, repo: RepoSettings) -> Self {
        Self { run, repo }
    }

    /// The per-run part.
    #[must_use]
    pub fn into_run(self) -> RunSettings {
        self.run
    }

    /// The repo-level part.
    #[must_use]
    pub fn into_repo(self) -> RepoSettings {
        self.repo
    }
}

/// How long run logs are kept, in days: the service's default and the most
/// a repo may ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    default_days: u32,
    max_days: u32,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            default_days: DEFAULT_RETENTION_DAYS,
            max_days: MAX_RETENTION_DAYS,
        }
    }
}

impl Retention {
    /// Keep logs `default_days` unless a repo asks otherwise, never more
    /// than `max_days`.
    #[must_use]
    pub fn new(default_days: u32, max_days: u32) -> Self {
        Self {
            default_days,
            max_days,
        }
    }

    /// Seconds a run's logs are kept when the repo asked for `requested`
    /// days (`ci.logRetentionDays`). At least one day.
    #[must_use]
    pub fn seconds(self, requested: Option<u32>) -> i64 {
        let days = requested
            .unwrap_or(self.default_days)
            .min(self.max_days)
            .max(1);
        i64::from(days) * SECONDS_PER_DAY
    }
}

/// Which cache a repo's built outputs go to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachePolicy {
    public: String,
    private_prefix: String,
}

impl CachePolicy {
    /// Caches are `public` and `<private_prefix><owner>`.
    #[must_use]
    pub fn new(public: String, private_prefix: String) -> Self {
        Self {
            public,
            private_prefix,
        }
    }

    /// The cache for an `owner`'s repo that asked for `requested`.
    /// Why: with no visibility known yet, outputs default to the owner's
    /// private cache, so nothing private leaks; a repo may opt into the
    /// public cache, but never into another owner's private one.
    ///
    /// # Errors
    ///
    /// When `requested` is neither the public cache nor the owner's.
    pub fn choose(&self, owner: &str, requested: Option<&str>) -> Result<String, InvalidConfig> {
        let own = format!("{}{owner}", self.private_prefix);
        match requested {
            None => Ok(own),
            Some(name) if name == own || name == self.public => Ok(String::from(name)),
            Some(name) => Err(InvalidConfig::new(format!(
                "ci.cache {name:?} must be {:?} or {own:?}",
                self.public
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CachePolicy, CheckSelection, Limits, Retention, RunSettings, Timeouts};

    #[test]
    fn logs_are_kept_as_asked_within_the_ceiling() {
        let day = 86_400;
        let retention = Retention::new(30, 90);
        assert_eq!(retention.seconds(None), 30 * day);
        assert_eq!(retention.seconds(Some(7)), 7 * day);
        assert_eq!(retention.seconds(Some(1000)), 90 * day);
        assert_eq!(retention.seconds(Some(0)), day, "at least a day");
    }

    #[test]
    fn outputs_go_to_the_owners_cache_unless_the_repo_opts_into_public() {
        let policy = CachePolicy::new(String::from("public"), String::from("private-"));
        assert_eq!(policy.choose("lucas", None).unwrap(), "private-lucas");
        assert_eq!(policy.choose("lucas", Some("public")).unwrap(), "public");
        assert_eq!(
            policy.choose("lucas", Some("private-lucas")).unwrap(),
            "private-lucas"
        );
        assert!(policy.choose("lucas", Some("private-alice")).is_err());
        assert!(policy.choose("lucas", Some("other")).is_err());
    }

    #[test]
    fn attribute_lists_are_validated() {
        let ok = vec![
            String::from("checks.x86_64-linux.unit"),
            String::from("packages.aarch64-darwin.default"),
            String::from("checks.x86_64-linux.it's_c++"),
        ];
        assert_eq!(
            CheckSelection::attrs(ok.clone()),
            Ok(CheckSelection::Attrs(ok))
        );
        for bad in ["", "checks..unit", "checks.x\".y", "a.${b}", "a b", "a\\b"] {
            assert!(
                CheckSelection::attrs(vec![String::from(bad)]).is_err(),
                "{bad:?} must be rejected"
            );
        }
        assert!(CheckSelection::attrs(Vec::new()).is_err());
    }

    #[test]
    fn timeouts_must_be_positive() {
        assert!(Timeouts::new(1, 1, 1).is_ok());
        assert!(Timeouts::new(0, 1, 1).is_err());
        assert!(Timeouts::new(1, 1, 0).is_err());
    }

    #[test]
    fn settings_are_capped_by_the_service() {
        let asked = RunSettings::new(
            CheckSelection::All,
            10,
            Timeouts::new(99_999, 99_999, 99_999).unwrap(),
        );
        let capped = asked.capped(Limits::new(2, 600, 1800));
        assert_eq!(capped.reruns(), 2);
        assert_eq!(capped.timeouts().check(), 600);
        assert_eq!(
            capped.timeouts().silent(),
            600,
            "never above the check timeout"
        );
        assert_eq!(capped.timeouts().run(), 1800);

        let modest = RunSettings::new(CheckSelection::All, 1, Timeouts::new(60, 30, 90).unwrap());
        assert_eq!(modest.clone().capped(Limits::new(2, 600, 1800)), modest);
    }
}
