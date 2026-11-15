//! [`Builder`] over the local Nix daemon: `nix-eval-jobs` to evaluate, one
//! `nix build --keep-going` to build. The daemon sends each derivation to a
//! remote builder of its system when the host's Nix config lists one, and
//! builds it on this host otherwise.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use super::internal_json::{self, NixLog};
use super::repos::bare_repo;
use super::systems;
use crate::application::ports::{BuildEvent, Builder, BuilderError, Evaluated};
use crate::domain::check::{Derivation, Systems};
use crate::domain::repo_config::{CheckSelection, Timeouts};
use crate::domain::run::NewRun;

/// Evaluation settings: the pusher's flake must not change Nix settings,
/// build during evaluation, or read anything but its own locked inputs.
/// Why: evaluating a flake runs the pusher's code on the CI host.
const EVAL_OPTIONS: [&str; 12] = [
    "--option",
    "accept-flake-config",
    "false",
    "--option",
    "allow-import-from-derivation",
    "false",
    "--option",
    "restrict-eval",
    "true",
    "--option",
    "pure-eval",
    "true",
];

/// What to evaluate: every `checks.<system>.<name>`, none when absent.
const SELECT_CHECKS: &str = "flake: flake.outputs.checks or { }";
/// Prefix that turns `nix-eval-jobs` attributes into flake attribute paths.
const CHECKS_PREFIX: &str = "checks.";
/// Name prefix of each entry of an explicit list, followed by its index.
const LISTED_PREFIX: &str = "c";
/// Output paths per `nix-store` call. Why: stay well under `ARG_MAX`.
const PATHS_PER_CALL: usize = 1000;
/// Most stderr kept to explain a failed command.
const STDERR_LIMIT: u64 = 64 * 1024;

/// Evaluates and builds through the local Nix daemon.
#[derive(Debug)]
pub struct NixBuilder {
    repos: PathBuf,
    eval_workers: u32,
    allowed_uris: String,
}

impl NixBuilder {
    /// Builder for bare repositories under `repos` (`<owner>/<name>.git`),
    /// evaluating with `eval_workers` processes. Evaluation may fetch only
    /// from the URI prefixes in `allowed_uris`.
    #[must_use]
    pub fn new(repos: PathBuf, eval_workers: u32, allowed_uris: &[String]) -> Self {
        Self {
            repos,
            eval_workers: eval_workers.max(1),
            // Why: Nix takes the list space-separated.
            allowed_uris: allowed_uris.join(" "),
        }
    }

    /// The flake reference of a run's commit.
    /// Why: `git+file://…?rev=` pins the exact commit, and works on bare
    /// repositories; `<path>#<sha>` would read the sha as an attribute.
    fn flake(&self, run: &NewRun) -> Result<String, BuilderError> {
        let path = bare_repo(&self.repos, run.repo()).map_err(BuilderError::Unavailable)?;
        Ok(format!(
            "git+file://{}?rev={}",
            path.display(),
            run.commit().as_str()
        ))
    }
}

/// The `nix-eval-jobs --select` function for a selection.
/// Why: an explicit list becomes one attribute per entry, named by its
/// index, so a missing attribute is an error on that entry alone. Entries
/// are validated attribute paths, and each segment is quoted.
fn select(selection: &CheckSelection) -> Cow<'static, str> {
    let CheckSelection::Attrs(attrs) = selection else {
        return Cow::Borrowed(SELECT_CHECKS);
    };
    let mut expr = String::from("flake: {");
    for (index, attr) in attrs.iter().enumerate() {
        let _ = write!(expr, " {LISTED_PREFIX}{index} = flake.outputs");
        for segment in attr.split('.') {
            let _ = write!(expr, ".\"{segment}\"");
        }
        let _ = write!(expr, " or (throw \"attribute '{attr}' not found\");");
    }
    expr.push_str(" }");
    Cow::Owned(expr)
}

/// The flake attribute a `nix-eval-jobs` attribute path stands for, and the
/// system its path names (for lines without a derivation).
fn name(selection: &CheckSelection, attr_path: &[Cow<'_, str>]) -> (String, String) {
    let mut segments = attr_path.iter().map(AsRef::as_ref);
    let first = segments.next().unwrap_or_default();
    let (mut attr, system) = match selection {
        CheckSelection::All => (format!("{CHECKS_PREFIX}{first}"), String::from(first)),
        CheckSelection::Attrs(attrs) => {
            let listed = first
                .strip_prefix(LISTED_PREFIX)
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(|index| attrs.get(index))
                .map_or(first, String::as_str);
            // Why: `checks.<system>.<name>` and friends name the system second.
            let system = listed.split('.').nth(1).unwrap_or_default();
            (String::from(listed), String::from(system))
        }
    };
    for segment in segments {
        attr.push('.');
        attr.push_str(segment);
    }
    (attr, system)
}

/// One line of `nix-eval-jobs` output.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvalLine<'a> {
    #[serde(default, borrow)]
    attr_path: Vec<Cow<'a, str>>,
    #[serde(default)]
    drv_path: Option<String>,
    #[serde(default)]
    outputs: BTreeMap<String, Option<String>>,
    #[serde(default)]
    system: Option<String>,
    #[serde(default, borrow)]
    cache_status: Option<Cow<'a, str>>,
    #[serde(default)]
    is_cached: bool,
    #[serde(default)]
    error: Option<String>,
}

impl EvalLine<'_> {
    fn into_evaluated(self, selection: &CheckSelection) -> Evaluated {
        let (attr, path_system) = name(selection, &self.attr_path);
        let system = self.system.unwrap_or(path_system);
        match (self.error, self.drv_path) {
            (None, Some(path)) => {
                // Why: `cached` = in a substituter, `local` = already on this
                // host; either way nothing needs building. Older versions
                // only report `isCached`.
                let cached = match self.cache_status.as_deref() {
                    Some(status) => matches!(status, "cached" | "local"),
                    None => self.is_cached,
                };
                let outputs = self.outputs.into_values().flatten().collect();
                Evaluated::Derivation {
                    attr,
                    system,
                    derivation: Derivation::new(path, outputs),
                    cached,
                }
            }
            (error, _) => Evaluated::Failed {
                attr,
                system,
                error: internal_json::strip_ansi(
                    error
                        .as_deref()
                        .unwrap_or("evaluation produced no derivation"),
                ),
            },
        }
    }
}

/// Start a command with piped output that dies with its handle.
fn spawn(command: &mut Command) -> Result<Child, BuilderError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Why: dropping a run (shutdown, timeout) must stop its Nix processes.
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| BuilderError::Unavailable(format!("{command:?}: {e}")))
}

/// Read at most [`STDERR_LIMIT`] bytes of a stream, then drain the rest.
fn read_capped(stream: impl AsyncRead + Unpin + Send) -> impl Future<Output = String> + Send {
    async move {
        let mut kept = Vec::new();
        let mut stream = stream;
        let mut head = (&mut stream).take(STDERR_LIMIT);
        let _ = head.read_to_end(&mut kept).await;
        let _ = tokio::io::copy(&mut stream, &mut tokio::io::sink()).await;
        String::from_utf8_lossy(&kept).into_owned()
    }
}

/// The part of `nix-eval-jobs` stderr worth showing: from the first error on.
/// Why: it also prints warnings about settings its bundled Nix does not know.
fn eval_failure(stderr: &str) -> String {
    let text = internal_json::strip_ansi(stderr);
    let start = text.find("error:").unwrap_or(0);
    String::from(text[start..].trim_end())
}

/// Output paths among `paths` that are not valid in the local store.
fn invalid_paths<'a>(
    paths: &'a [&'a str],
) -> impl Future<Output = Result<HashSet<String>, BuilderError>> + Send + 'a {
    async move {
        let mut invalid = HashSet::new();
        for chunk in paths.chunks(PATHS_PER_CALL) {
            let output = Command::new("nix-store")
                .args(["--check-validity", "--print-invalid"])
                .args(chunk)
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output()
                .await
                .map_err(|e| BuilderError::Unavailable(format!("nix-store: {e}")))?;
            if !output.status.success() {
                return Err(BuilderError::Unavailable(format!(
                    "nix-store --check-validity: {}",
                    String::from_utf8_lossy(&output.stderr).trim_end()
                )));
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            invalid.extend(stdout.lines().map(String::from));
        }
        Ok(invalid)
    }
}

/// When a running build started and last printed.
struct Timing {
    started: Instant,
    last_output: Instant,
}

impl Timing {
    /// Whether a build ending now ran into `timeouts`.
    /// Why: Nix's "timed out" message does not say which derivation, so it
    /// is told by time: Nix kills a build once it has run `check` seconds
    /// (measured from a start event that precedes it, so never early here)
    /// or printed nothing for `silent` seconds (allowing a second for lines
    /// reaching us later than Nix read them).
    fn timed_out(&self, now: Instant, timeouts: Timeouts) -> bool {
        let ran = now.duration_since(self.started);
        let quiet = now.duration_since(self.last_output) + Duration::from_secs(1);
        ran >= Duration::from_secs(timeouts.check())
            || quiet >= Duration::from_secs(timeouts.silent())
    }
}

/// Forward `nix build` stderr as build events until it closes.
fn forward(
    stderr: impl AsyncRead + Unpin + Send,
    timeouts: Timeouts,
    events: mpsc::Sender<BuildEvent>,
) -> impl Future<Output = ()> + Send {
    async move {
        let mut builds: HashMap<u64, Timing> = HashMap::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let now = Instant::now();
            let event = match internal_json::parse(&line) {
                NixLog::BuildStarted { id, drv, machine } => {
                    let timing = Timing {
                        started: now,
                        last_output: now,
                    };
                    builds.insert(id, timing);
                    BuildEvent::Started {
                        build: id,
                        drv: drv.into_owned(),
                        machine: machine.into_owned(),
                    }
                }
                NixLog::BuildLine { id, text } => {
                    if let Some(timing) = builds.get_mut(&id) {
                        timing.last_output = now;
                    }
                    BuildEvent::Line {
                        build: id,
                        text: text.into_owned(),
                    }
                }
                NixLog::Stopped { id } => match builds.remove(&id) {
                    Some(timing) if timing.timed_out(now, timeouts) => {
                        BuildEvent::TimedOut { build: id }
                    }
                    _ => continue,
                },
                NixLog::Message(text) => BuildEvent::Message(text),
                NixLog::Ignored => continue,
            };
            // Why: keep draining even if nobody listens, or Nix blocks.
            let _ = events.send(event).await;
        }
    }
}

impl Builder for NixBuilder {
    fn systems(&self) -> impl Future<Output = Result<Systems, BuilderError>> + Send {
        async move {
            let output = Command::new("nix")
                .args(["config", "show", "--json"])
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output()
                .await
                .map_err(|e| BuilderError::Unavailable(format!("nix config show: {e}")))?;
            if !output.status.success() {
                return Err(BuilderError::Unavailable(format!(
                    "nix config show: {}",
                    String::from_utf8_lossy(&output.stderr).trim_end()
                )));
            }
            let json = String::from_utf8_lossy(&output.stdout);
            let read = |path: &str| {
                std::fs::read_to_string(Path::new(path)).map_err(|e| format!("{path}: {e}"))
            };
            systems::parse(&json, read)
                .map(Systems::new)
                .map_err(BuilderError::Unavailable)
        }
    }

    fn evaluate(
        &self,
        run: &NewRun,
        selection: &CheckSelection,
    ) -> impl Future<Output = Result<Vec<Evaluated>, BuilderError>> + Send {
        let flake = self.flake(run);
        async move {
            let flake = flake?;
            let workers = self.eval_workers.to_string();
            let select = select(selection);
            let mut child = spawn(
                Command::new("nix-eval-jobs")
                    .args(["--flake", &flake, "--select", &select])
                    .args(["--force-recurse", "--check-cache-status"])
                    .args(["--workers", &workers])
                    .args(EVAL_OPTIONS)
                    .args(["--option", "allowed-uris", &self.allowed_uris]),
            )?;
            let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
                return Err(BuilderError::Unavailable(String::from(
                    "nix-eval-jobs: no pipes",
                )));
            };
            let read_stdout = async {
                let mut evaluated = Vec::new();
                let mut lines = BufReader::new(stdout).lines();
                while let Some(line) = lines.next_line().await? {
                    match serde_json::from_str::<EvalLine<'_>>(&line) {
                        Ok(parsed) => evaluated.push(parsed.into_evaluated(selection)),
                        Err(e) => tracing::warn!(%e, line, "unreadable nix-eval-jobs line"),
                    }
                }
                Ok::<_, std::io::Error>(evaluated)
            };
            let (evaluated, stderr) = tokio::join!(read_stdout, read_capped(stderr));
            let status = child
                .wait()
                .await
                .map_err(|e| BuilderError::Unavailable(format!("nix-eval-jobs: {e}")))?;
            let evaluated =
                evaluated.map_err(|e| BuilderError::Unavailable(format!("nix-eval-jobs: {e}")))?;
            // Why: per-attribute errors come back as lines with exit 0; a
            // failing exit means the flake itself could not be evaluated.
            if !status.success() {
                return Err(BuilderError::Flake(eval_failure(&stderr)));
            }
            Ok(evaluated)
        }
    }

    fn build(
        &self,
        targets: &[&Derivation],
        timeouts: Timeouts,
        events: mpsc::Sender<BuildEvent>,
    ) -> impl Future<Output = Result<Vec<bool>, BuilderError>> + Send {
        async move {
            let installables: Vec<String> = targets
                .iter()
                .map(|drv| format!("{}^*", drv.path()))
                .collect();
            // Why: the daemon honours both limits from untrusted clients,
            // and passes them on to remote builders.
            let check = timeouts.check().to_string();
            let silent = timeouts.silent().to_string();
            let mut child = spawn(
                Command::new("nix")
                    .args(["build", "--keep-going", "--no-link"])
                    .args(["--log-format", "internal-json"])
                    .args(["--timeout", &check, "--max-silent-time", &silent])
                    .args(&installables),
            )?;
            let Some(stderr) = child.stderr.take() else {
                return Err(BuilderError::Unavailable(String::from(
                    "nix build: no stderr",
                )));
            };
            drop(child.stdout.take());
            forward(stderr, timeouts, events).await;
            // Why: a failing exit is normal with failing checks; which
            // targets passed is read from the store below.
            child
                .wait()
                .await
                .map_err(|e| BuilderError::Unavailable(format!("nix build: {e}")))?;

            let outputs: Vec<&str> = targets
                .iter()
                .flat_map(|drv| drv.outputs().iter().map(String::as_str))
                .collect();
            let invalid = invalid_paths(&outputs).await?;
            Ok(targets
                .iter()
                .map(|drv| {
                    !drv.outputs().is_empty()
                        && drv.outputs().iter().all(|out| !invalid.contains(out))
                })
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{EvalLine, Timing, eval_failure, select};
    use crate::application::ports::Evaluated;
    use crate::domain::repo_config::{CheckSelection, Timeouts};

    fn evaluated(line: &str) -> Evaluated {
        evaluated_as(line, &CheckSelection::All)
    }

    fn evaluated_as(line: &str, selection: &CheckSelection) -> Evaluated {
        serde_json::from_str::<EvalLine<'_>>(line)
            .unwrap()
            .into_evaluated(selection)
    }

    fn listed() -> CheckSelection {
        CheckSelection::attrs(vec![
            String::from("packages.x86_64-linux.default"),
            String::from("checks.aarch64-linux.it's"),
        ])
        .unwrap()
    }

    #[test]
    fn explicit_lists_become_indexed_attributes() {
        assert_eq!(
            select(&listed()),
            "flake: { c0 = flake.outputs.\"packages\".\"x86_64-linux\".\"default\" \
             or (throw \"attribute 'packages.x86_64-linux.default' not found\"); \
             c1 = flake.outputs.\"checks\".\"aarch64-linux\".\"it's\" \
             or (throw \"attribute 'checks.aarch64-linux.it's' not found\"); }"
        );
        assert_eq!(
            select(&CheckSelection::All),
            "flake: flake.outputs.checks or { }"
        );

        let line = r#"{"attr":"c0","attrPath":["c0"],"cacheStatus":"cached","drvPath":"/nix/store/d.drv","outputs":{"out":"/nix/store/o"},"system":"x86_64-linux"}"#;
        assert!(matches!(
            evaluated_as(line, &listed()),
            Evaluated::Derivation { attr, cached: true, .. } if attr == "packages.x86_64-linux.default"
        ));
        let missing = r#"{"attr":"c1.sub","attrPath":["c1","sub"],"error":"attribute 'checks.aarch64-linux.it's' not found"}"#;
        assert!(matches!(
            evaluated_as(missing, &listed()),
            Evaluated::Failed { attr, system, .. }
                if attr == "checks.aarch64-linux.it's.sub" && system == "aarch64-linux"
        ));
    }

    #[test]
    fn timeouts_are_told_by_time() {
        let timeouts = Timeouts::new(10, 5, 100).unwrap();
        let start = Instant::now();
        let timing = |ran: u64, quiet: u64| Timing {
            started: start,
            last_output: start + Duration::from_secs(ran - quiet),
        };
        let at = |secs: u64| start + Duration::from_secs(secs);
        assert!(!timing(3, 1).timed_out(at(3), timeouts));
        assert!(timing(10, 0).timed_out(at(10), timeouts), "ran too long");
        assert!(timing(8, 4).timed_out(at(8), timeouts), "silent too long");
        assert!(!timing(8, 3).timed_out(at(8), timeouts));
    }

    #[test]
    fn derivation_lines_become_checks() {
        let line = r#"{"attr":"x86_64-linux.pass","attrPath":["x86_64-linux","pass"],"cacheStatus":"notBuilt","drvPath":"/nix/store/n-pass.drv","isCached":false,"name":"pass","neededBuilds":[],"neededSubstitutes":[],"outputs":{"out":"/nix/store/j-pass"},"requiredSystemFeatures":[],"system":"x86_64-linux"}"#;
        let Evaluated::Derivation {
            attr,
            system,
            derivation,
            cached,
        } = evaluated(line)
        else {
            panic!("not a derivation");
        };
        assert_eq!(attr, "checks.x86_64-linux.pass");
        assert_eq!(system, "x86_64-linux");
        assert_eq!(derivation.path(), "/nix/store/n-pass.drv");
        assert_eq!(derivation.outputs(), ["/nix/store/j-pass"]);
        assert!(!cached);

        for status in ["cached", "local"] {
            let line = format!(
                r#"{{"attr":"a.b","attrPath":["a","b"],"cacheStatus":"{status}","drvPath":"/nix/store/d.drv","outputs":{{"out":"/nix/store/o"}},"system":"a"}}"#
            );
            assert!(matches!(
                evaluated(&line),
                Evaluated::Derivation { cached: true, .. }
            ));
        }
    }

    #[test]
    fn error_lines_become_failed_checks() {
        let line = r#"{"attr":"x86_64-linux.evalerr","attrPath":["x86_64-linux","evalerr"],"error":"error:\n       \u001b[31;1mbad check\u001b[0m"}"#;
        assert_eq!(
            evaluated(line),
            Evaluated::Failed {
                attr: String::from("checks.x86_64-linux.evalerr"),
                system: String::from("x86_64-linux"),
                error: String::from("error:\n       bad check"),
            }
        );
    }

    #[test]
    fn flake_failures_skip_warnings() {
        let stderr = "warning: unknown setting 'allowed-users'\n\u{1b}[31;1merror:\u{1b}[0m syntax error\n   at flake.nix:2:1\n";
        assert_eq!(
            eval_failure(stderr),
            "error: syntax error\n   at flake.nix:2:1"
        );
    }
}
