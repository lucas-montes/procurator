//! Push what a commit builds to a binary cache.
//!
//! Builds the commit's `packages` and `checks` for this system, then uploads
//! every path this machine built itself (rather than substituted) that those
//! outputs need — build-time dependencies included — so CI and teammates
//! substitute them instead of rebuilding.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::Deserialize;

use super::cache::CacheError;

/// Lists `<flake>#<attr>` for every package and check of this system.
/// Why: the flake reference comes through the environment so no path ever
/// needs escaping inside a Nix string.
const OUTPUTS_EXPR: &str = r#"
let
  ref = builtins.getEnv "PCR_FLAKE_REF";
  flake = builtins.getFlake ref;
  system = builtins.currentSystem;
  attrs = output:
    map (name: "${ref}#${output}.${system}.${name}")
      (builtins.attrNames (flake.${output}.${system} or { }));
in
attrs "packages" ++ attrs "checks"
"#;

/// Store paths per `nix-store` invocation. Why: stay well under `ARG_MAX`.
const ARGS_PER_CALL: usize = 1000;

/// One entry of `nix build --json`.
#[derive(Deserialize)]
struct Built<'a> {
    #[serde(borrow, rename = "drvPath")]
    drv_path: &'a str,
}

/// The part of `nix path-info --json` this module reads.
#[derive(Deserialize)]
struct PathInfo {
    #[serde(default)]
    ultimate: bool,
}

/// One entry of the list form of `nix path-info --json` (Nix before 2.19).
#[derive(Deserialize)]
struct ListedPathInfo<'a> {
    #[serde(borrow)]
    path: &'a str,
    #[serde(default)]
    ultimate: bool,
}

/// `nix path-info --json` output: keyed by path since Nix 2.19, a list before.
#[derive(Deserialize)]
#[serde(untagged)]
enum PathInfos<'a> {
    #[serde(borrow)]
    Keyed(BTreeMap<&'a str, Option<PathInfo>>),
    #[serde(borrow)]
    Listed(Vec<ListedPathInfo<'a>>),
}

/// Build what commit `rev` of the flake at `repo` exposes for this system and
/// push the locally built part of it to `url`.
///
/// Returns how many locally built paths are now in the cache, uploaded by
/// this call or already there (0 when the flake exposes
/// nothing for this system or everything was substituted).
///
/// # Errors
/// Fails when the repository path cannot be resolved, or a Nix command
/// fails or prints output this function cannot read.
pub fn push_commit(repo: &Path, rev: &str, url: &str) -> Result<usize, CacheError> {
    let flake = format!("git+file://{}?rev={rev}", repo.canonicalize()?.display());

    let listed = run(
        nix()
            .args(["eval", "--impure", "--json", "--expr", OUTPUTS_EXPR])
            .env("PCR_FLAKE_REF", &flake),
        None,
        false,
    )?;
    let installables: Vec<String> = serde_json::from_str(&listed)?;
    if installables.is_empty() {
        return Ok(0);
    }

    // Why: progress goes to the terminal; a long build must not look hung.
    let built = run(
        nix()
            .args(["build", "--no-link", "--json"])
            .args(&installables),
        None,
        true,
    )?;
    let built: Vec<Built<'_>> = serde_json::from_str(&built)?;

    // Every derivation the build needed, then their outputs that exist here.
    let closure = store_query("--requisites", built.iter().map(|b| b.drv_path))?;
    let drvs = closure
        .paths()
        .filter(|p| Path::new(p).extension().is_some_and(|ext| ext == "drv"));
    let outputs = store_query("--outputs", drvs)?;
    let present = outputs.paths().filter(|p| Path::new(p).exists());
    // Why: an output can reference a locally added source; include those.
    let runtime = store_query("--requisites", present)?;
    let candidates: BTreeSet<&str> = runtime.paths().collect();

    let infos = run(
        nix().args(["path-info", "--json", "--stdin"]),
        Some(&joined(candidates.iter().copied())),
        false,
    )?;
    let local = locally_built(&infos)?;
    if local.is_empty() {
        return Ok(0);
    }

    run(
        nix().args(["copy", "--no-recursive", "--stdin", "--to", url]),
        Some(&joined(local.iter().copied())),
        true,
    )?;
    Ok(local.len())
}

/// `nix` with the commands this module needs enabled.
fn nix() -> Command {
    let mut command = Command::new("nix");
    command.args(["--extra-experimental-features", "nix-command flakes"]);
    command
}

/// Paths from `nix path-info --json` that this machine built or added itself.
fn locally_built(json: &str) -> Result<BTreeSet<&str>, CacheError> {
    Ok(match serde_json::from_str(json)? {
        PathInfos::Keyed(infos) => infos
            .into_iter()
            .filter(|(_, info)| info.as_ref().is_some_and(|i| i.ultimate))
            .map(|(path, _)| path)
            .collect(),
        PathInfos::Listed(infos) => infos
            .into_iter()
            .filter(|info| info.ultimate)
            .map(|info| info.path)
            .collect(),
    })
}

/// Output of a batched `nix-store --query`: one stdout per batch.
struct QueryOutput(Vec<String>);

impl QueryOutput {
    /// Every printed path; may repeat across batches.
    fn paths(&self) -> impl Iterator<Item = &str> {
        self.0.iter().flat_map(|out| out.lines())
    }
}

/// Run `nix-store --query <flag>` over `paths` in batches.
fn store_query<'a>(
    flag: &str,
    paths: impl Iterator<Item = &'a str>,
) -> Result<QueryOutput, CacheError> {
    let paths: BTreeSet<&str> = paths.collect();
    let paths: Vec<&str> = paths.into_iter().collect();
    paths
        .chunks(ARGS_PER_CALL)
        .map(|batch| {
            run(
                Command::new("nix-store")
                    .args(["--query", flag])
                    .args(batch),
                None,
                false,
            )
        })
        .collect::<Result<_, _>>()
        .map(QueryOutput)
}

/// One path per line, for `--stdin`.
fn joined<'a>(paths: impl Iterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for path in paths {
        out.push_str(path);
        out.push('\n');
    }
    out
}

/// Run `command`, feeding it `input`, and return its stdout.
/// With `show_progress`, stderr goes to the terminal instead of the error.
fn run(
    command: &mut Command,
    input: Option<&str>,
    show_progress: bool,
) -> Result<String, CacheError> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(if show_progress {
            Stdio::inherit()
        } else {
            Stdio::piped()
        });
    let mut child = command.spawn()?;
    let stdin = child.stdin.take();
    // Why: write on another thread so a full stdout pipe cannot deadlock us.
    let output = std::thread::scope(|scope| {
        if let (Some(mut stdin), Some(input)) = (stdin, input) {
            scope.spawn(move || {
                let _ = stdin.write_all(input.as_bytes());
            });
        }
        child.wait_with_output()
    })?;
    if !output.status.success() {
        return Err(CacheError::NixCommandFailed {
            exit_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    String::from_utf8(output.stdout).map_err(|_| CacheError::NixCommandFailed {
        exit_code: output.status.code(),
        stderr: String::from("output is not UTF-8"),
    })
}

#[cfg(test)]
mod tests {
    use super::locally_built;

    #[test]
    fn reads_keyed_path_info() {
        let json = r#"{
            "/nix/store/a-built": {"ultimate": true, "narSize": 1},
            "/nix/store/b-substituted": {"ultimate": false},
            "/nix/store/c-invalid": null
        }"#;
        let local = locally_built(json).unwrap();
        assert_eq!(
            local.into_iter().collect::<Vec<_>>(),
            ["/nix/store/a-built"]
        );
    }

    #[test]
    fn reads_listed_path_info() {
        let json = r#"[
            {"path": "/nix/store/a-built", "ultimate": true},
            {"path": "/nix/store/b-substituted"}
        ]"#;
        let local = locally_built(json).unwrap();
        assert_eq!(
            local.into_iter().collect::<Vec<_>>(),
            ["/nix/store/a-built"]
        );
    }
}
