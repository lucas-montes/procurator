//! [`ConfigSource`] for `procurator.nix`: read the blob from the bare repo
//! with git, evaluate it in isolation with Nix, validate it, and keep the
//! result in memory keyed by the blob id.
//! Why: most pushes do not touch `procurator.nix`, so its blob id is
//! unchanged and no Nix runs at all.

use std::collections::HashMap;
use std::future::Future;
use std::mem;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;

use super::internal_json::strip_ansi;
use super::repos::bare_repo;
use crate::application::ports::{ConfigSource, RepoConfigError};
use crate::domain::event::{EventKind, Webhook};
use crate::domain::repo_config::{CheckSelection, RepoConfig, RepoSettings, RunSettings, Timeouts};
use crate::domain::run::{NewRun, RepoName};

/// Most webhooks a repo may list.
const MAX_WEBHOOKS: usize = 10;

/// The file, at the repository root.
const FILE: &str = "procurator.nix";
/// Largest accepted file. Why: it is a small attribute set, never code.
const MAX_BYTES: u64 = 64 * 1024;
/// Longest an evaluation may take.
const EVAL_TIMEOUT: Duration = Duration::from_secs(30);
/// Parsed configs kept; the map is emptied when it grows past this.
const CACHE_ENTRIES: usize = 256;
/// Evaluation settings: the file may not import, fetch or read anything.
const EVAL_OPTIONS: [&str; 12] = [
    "--option",
    "restrict-eval",
    "true",
    "--option",
    "allowed-uris",
    "",
    "--option",
    "allow-import-from-derivation",
    "false",
    "--option",
    "pure-eval",
    "true",
];

/// Reads `procurator.nix` from bare repositories under a base path.
#[derive(Debug)]
pub struct GitConfigSource {
    repos: PathBuf,
    parsed: Mutex<HashMap<String, RepoConfig>>,
}

impl GitConfigSource {
    /// A source for bare repositories under `repos` (`<owner>/<name>.git`).
    #[must_use]
    pub fn new(repos: PathBuf) -> Self {
        Self {
            repos,
            parsed: Mutex::new(HashMap::new()),
        }
    }

    fn cached(&self, blob: &str) -> Option<RepoConfig> {
        let parsed = self.parsed.lock().unwrap_or_else(PoisonError::into_inner);
        parsed.get(blob).cloned()
    }

    fn remember(&self, blob: String, config: RepoConfig) {
        let mut parsed = self.parsed.lock().unwrap_or_else(PoisonError::into_inner);
        if parsed.len() >= CACHE_ENTRIES {
            parsed.clear();
        }
        parsed.insert(blob, config);
    }
}

/// Run git against a bare repo; its stdout, or `None` when it exits 1
/// (`--verify --quiet` found nothing).
fn git<'a>(
    repo: &'a Path,
    args: &'a [&'a str],
) -> impl Future<Output = Result<Option<Vec<u8>>, RepoConfigError>> + Send + 'a {
    async move {
        let output = Command::new("git")
            .arg("--git-dir")
            .arg(repo)
            .args(args)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| RepoConfigError::Unavailable(format!("git: {e}")))?;
        match output.status.code() {
            Some(0) => Ok(Some(output.stdout)),
            Some(1) => Ok(None),
            _ => Err(RepoConfigError::Unavailable(format!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim_end()
            ))),
        }
    }
}

/// Evaluate the file's text to JSON, with nothing reachable from it.
fn evaluate(text: &str) -> impl Future<Output = Result<String, RepoConfigError>> + Send + '_ {
    async move {
        let output = Command::new("nix")
            .args(["eval", "--json", "--expr", text])
            .args(EVAL_OPTIONS)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output();
        let output = tokio::time::timeout(EVAL_TIMEOUT, output)
            .await
            .map_err(|_| {
                RepoConfigError::Invalid(format!(
                    "{FILE}: evaluation took longer than {} seconds",
                    EVAL_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|e| RepoConfigError::Unavailable(format!("nix eval: {e}")))?;
        if !output.status.success() {
            let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
            let start = stderr.find("error:").unwrap_or(0);
            return Err(RepoConfigError::Invalid(format!(
                "{FILE}: {}",
                stderr[start..].trim_end()
            )));
        }
        String::from_utf8(output.stdout)
            .map_err(|e| RepoConfigError::Unavailable(format!("nix eval output: {e}")))
    }
}

/// The file's shape. Unknown keys are errors, so typos do not pass silently.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileDto {
    #[serde(default)]
    ci: CiDto,
    #[serde(default)]
    #[expect(dead_code, reason = "validated now, read by the deploy step")]
    deploy: Option<DeployDto>,
}

/// `ci`. Repo-level keys are checked now and used by later steps.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CiDto {
    checks: Option<ChecksDto>,
    reruns: Option<u32>,
    timeouts: Option<TimeoutsDto>,
    log_retention_days: Option<u32>,
    cache: Option<String>,
    #[serde(default)]
    webhooks: Vec<WebhookDto>,
}

/// `ci.checks`: `"all"` or a list of attribute paths.
#[derive(Deserialize)]
#[serde(untagged)]
enum ChecksDto {
    Keyword(String),
    List(Vec<String>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TimeoutsDto {
    check: Option<u64>,
    silent: Option<u64>,
    run: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WebhookDto {
    url_secret: String,
    events: Vec<EventKind>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code, reason = "used by the deploy step")]
struct DeployDto {
    url: Option<String>,
    branch: Option<String>,
}

/// Validate evaluated JSON into a repo config.
fn parse(json: &str) -> Result<RepoConfig, RepoConfigError> {
    let invalid = |reason: String| RepoConfigError::Invalid(format!("{FILE}: {reason}"));
    // Why: serde also reads a struct from a list; the file must be a set.
    if !json.trim_start().starts_with('{') {
        return Err(invalid(String::from("must be an attribute set")));
    }
    let file: FileDto = serde_json::from_str(json).map_err(|e| invalid(e.to_string()))?;
    let mut ci = file.ci;
    if ci.webhooks.len() > MAX_WEBHOOKS {
        return Err(invalid(format!(
            "ci.webhooks lists more than {MAX_WEBHOOKS}"
        )));
    }
    let webhooks = mem::take(&mut ci.webhooks)
        .into_iter()
        .map(|hook| Webhook::new(hook.url_secret, hook.events))
        .collect::<Result<Vec<_>, _>>()
        .map_err(invalid)?;
    let repo = RepoSettings::new(ci.cache.take(), webhooks).keeping_logs(ci.log_retention_days);
    let checks = match ci.checks {
        None => CheckSelection::All,
        Some(ChecksDto::Keyword(keyword)) if keyword == "all" => CheckSelection::All,
        Some(ChecksDto::Keyword(other)) => {
            return Err(invalid(format!(
                "ci.checks must be \"all\" or a list, not {other:?}"
            )));
        }
        Some(ChecksDto::List(attrs)) => {
            CheckSelection::attrs(attrs).map_err(|e| RepoConfigError::Invalid(e.to_string()))?
        }
    };
    let defaults = Timeouts::default();
    let timeouts = match ci.timeouts {
        None => defaults,
        Some(given) => Timeouts::new(
            given.check.unwrap_or(defaults.check()),
            given.silent.unwrap_or(defaults.silent()),
            given.run.unwrap_or(defaults.run()),
        )
        .map_err(|e| RepoConfigError::Invalid(e.to_string()))?,
    };
    let run = RunSettings::new(checks, ci.reruns.unwrap_or(0), timeouts);
    Ok(RepoConfig::new(run, repo))
}

impl GitConfigSource {
    /// The config of `procurator.nix` at `rev` (a commit or ref) of a bare
    /// repo; the defaults when the file or `rev` does not exist.
    async fn config_at(&self, repo: &Path, rev: &str) -> Result<RepoConfig, RepoConfigError> {
        let spec = format!("{rev}:{FILE}");
        let found = git(repo, &["rev-parse", "--verify", "--quiet", &spec]).await?;
        let Some(blob) = found else {
            return Ok(RepoConfig::default());
        };
        let blob = String::from(String::from_utf8_lossy(&blob).trim());
        if let Some(config) = self.cached(&blob) {
            return Ok(config);
        }

        let size = git(repo, &["cat-file", "-s", &blob])
            .await?
            .unwrap_or_default();
        let size: u64 = String::from_utf8_lossy(&size)
            .trim()
            .parse()
            .unwrap_or(u64::MAX);
        if size > MAX_BYTES {
            return Err(RepoConfigError::Invalid(format!(
                "{FILE}: larger than {MAX_BYTES} bytes"
            )));
        }
        let text = git(repo, &["cat-file", "blob", &blob])
            .await?
            .unwrap_or_default();
        let text = String::from_utf8(text)
            .map_err(|_| RepoConfigError::Invalid(format!("{FILE}: not UTF-8")))?;
        let config = parse(&evaluate(&text).await?)?;
        self.remember(blob, config.clone());
        Ok(config)
    }
}

impl ConfigSource for GitConfigSource {
    fn run_settings(
        &self,
        run: &NewRun,
    ) -> impl Future<Output = Result<RunSettings, RepoConfigError>> + Send {
        async move {
            let repo = bare_repo(&self.repos, run.repo()).map_err(RepoConfigError::Unavailable)?;
            let config = self.config_at(&repo, run.commit().as_str()).await?;
            Ok(config.into_run())
        }
    }

    fn repo_settings(
        &self,
        repo: &RepoName,
    ) -> impl Future<Output = Result<RepoSettings, RepoConfigError>> + Send {
        async move {
            let path = bare_repo(&self.repos, repo).map_err(RepoConfigError::Unavailable)?;
            // Why: HEAD of a bare repo names the default branch; the hook
            // points it at the first branch pushed.
            let config = self.config_at(&path, "HEAD").await?;
            let head = git(&path, &["symbolic-ref", "--quiet", "HEAD"]).await?;
            let settings = config.into_repo();
            Ok(match head {
                Some(name) => {
                    settings.on_branch(String::from(String::from_utf8_lossy(&name).trim()))
                }
                None => settings,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::application::ports::RepoConfigError;
    use crate::domain::event::EventKind;
    use crate::domain::repo_config::{CheckSelection, RepoConfig, Timeouts};

    #[test]
    fn empty_means_defaults() {
        assert_eq!(parse("{}").unwrap(), RepoConfig::default());
        assert_eq!(parse(r#"{"ci":{}}"#).unwrap(), RepoConfig::default());
    }

    #[test]
    fn the_cache_is_a_repo_level_setting() {
        let config = parse(r#"{"ci":{"cache":"public","reruns":1}}"#).unwrap();
        assert_eq!(config.clone().into_repo().cache(), Some("public"));
        assert_eq!(config.into_run().reruns(), 1);
    }

    #[test]
    fn webhooks_are_parsed_and_checked() {
        let config = parse(
            r#"{"ci":{"webhooks":[{"urlSecret":"slack","events":["run.failed","check.failed"]}]}}"#,
        )
        .unwrap();
        let repo = config.into_repo();
        assert_eq!(repo.webhooks().len(), 1);
        assert_eq!(repo.webhooks()[0].url_secret(), "slack");
        assert!(repo.webhooks()[0].wants(EventKind::CheckFailed));
        assert!(!repo.webhooks()[0].wants(EventKind::RunSucceeded));

        let bad_name = r#"{"ci":{"webhooks":[{"urlSecret":"https://x","events":["run.failed"]}]}}"#;
        assert!(matches!(parse(bad_name), Err(RepoConfigError::Invalid(_))));
        let no_events = r#"{"ci":{"webhooks":[{"urlSecret":"x","events":[]}]}}"#;
        assert!(matches!(parse(no_events), Err(RepoConfigError::Invalid(_))));
    }

    #[test]
    fn the_documented_example_parses() {
        let json = r#"{
            "ci": {
                "checks": ["checks.x86_64-linux.unit", "packages.x86_64-linux.default"],
                "reruns": 2,
                "timeouts": {"check": 600, "silent": 60},
                "logRetentionDays": 30,
                "cache": null,
                "webhooks": [{"urlSecret": "slack", "events": ["run.failed", "run.recovered"]}]
            },
            "deploy": {"url": null, "branch": "main"}
        }"#;
        assert_eq!(
            parse(json).unwrap().into_repo().log_retention_days(),
            Some(30)
        );
        let settings = parse(json).unwrap().into_run();
        assert_eq!(settings.reruns(), 2);
        assert_eq!(settings.timeouts(), Timeouts::new(600, 60, 7200).unwrap());
        assert_eq!(
            settings.checks(),
            &CheckSelection::Attrs(vec![
                String::from("checks.x86_64-linux.unit"),
                String::from("packages.x86_64-linux.default"),
            ])
        );
        assert_eq!(
            parse(r#"{"ci":{"checks":"all"}}"#)
                .unwrap()
                .into_run()
                .checks(),
            &CheckSelection::All
        );
    }

    #[test]
    fn mistakes_are_rejected_with_a_reason() {
        for (json, needle) in [
            (r#"{"ci":{"rerun":1}}"#, "unknown field `rerun`"),
            (r#"{"cii":{}}"#, "unknown field `cii`"),
            (r#"{"ci":{"checks":"some"}}"#, "\"some\""),
            (r#"{"ci":{"checks":["a.\"b"]}}"#, "attribute path"),
            (r#"{"ci":{"timeouts":{"run":0}}}"#, "at least 1"),
            (r#"{"ci":{"reruns":-1}}"#, "invalid value"),
            (
                r#"{"ci":{"webhooks":[{"urlSecret":"x","events":["run.exploded"]}]}}"#,
                "run.exploded",
            ),
            ("[]", "procurator.nix"),
        ] {
            match parse(json) {
                Err(RepoConfigError::Invalid(reason)) => {
                    assert!(reason.contains(needle), "{json}: got {reason}");
                }
                other => panic!("{json}: expected invalid, got {other:?}"),
            }
        }
    }
}
