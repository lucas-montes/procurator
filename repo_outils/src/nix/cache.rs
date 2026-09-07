<<<<<<< HEAD
//! Nix cache operations: read config, push/pull derivations.

use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{error, info, warn};

/// Errors for Nix cache operations.
#[derive(Debug)]
pub enum CacheError {
    Io(std::io::Error),
    JsonParse(serde_json::Error),
    NixCommandFailed { stderr: String },
    NoCacheConfig,
    NoArtifacts,
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CacheError::Io(e) => write!(f, "IO error: {}", e),
            CacheError::JsonParse(e) => write!(f, "JSON parse error: {}", e),
            CacheError::NixCommandFailed { stderr } => {
                write!(f, "Nix command failed: {}", stderr)
            }
            CacheError::NoCacheConfig => write!(f, "No cache URL configured"),
            CacheError::NoArtifacts => write!(f, "No Nix artifacts found"),
=======
//! Nix binary cache operations.
//!
//! Reads cache URL from flake `nixConfig` and pushes store paths
//! to a remote binary cache via `nix copy`.

use std::fmt;
use std::path::Path;
use std::process::Command;
use tracing::warn;

/// Errors specific to cache operations.
#[derive(Debug)]
pub enum CacheError {
    /// Underlying IO error (command not found, etc.)
    Io(std::io::Error),
    /// `nix eval` / `nix copy` exited non-zero
    NixCommandFailed {
        exit_code: Option<i32>,
        stderr: String,
    },
    /// JSON from `nix eval` could not be parsed
    JsonParse(serde_json::Error),
    /// No substituter URL found in flake config
    NoCacheConfig,
    /// No Nix build artifacts found (no `./result` symlinks)
    NoArtifacts,
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::Io(err) => write!(f, "IO error: {}", err),
            CacheError::NixCommandFailed { exit_code, stderr } => {
                write!(
                    f,
                    "Nix command failed (exit {:?}): {}",
                    exit_code,
                    stderr.trim()
                )
            }
            CacheError::JsonParse(err) => write!(f, "Failed to parse JSON: {}", err),
            CacheError::NoCacheConfig => write!(f, "No cache URL configured in flake nixConfig"),
            CacheError::NoArtifacts => write!(f, "No Nix build artifacts found (no ./result)"),
>>>>>>> master
        }
    }
}

<<<<<<< HEAD
impl std::error::Error for CacheError {}
=======
impl std::error::Error for CacheError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CacheError::Io(err) => Some(err),
            CacheError::JsonParse(err) => Some(err),
            _ => None,
        }
    }
}
>>>>>>> master

impl From<std::io::Error> for CacheError {
    fn from(err: std::io::Error) -> Self {
        CacheError::Io(err)
    }
}

impl From<serde_json::Error> for CacheError {
    fn from(err: serde_json::Error) -> Self {
        CacheError::JsonParse(err)
    }
}

<<<<<<< HEAD
pub type Result<T> = std::result::Result<T, CacheError>;

/// Nix config output from `nix eval -f flake.nix nixConfig.extra-substituters`.
#[derive(Debug, Deserialize)]
struct NixConfig {
    #[serde(rename = "extra-substituters")]
    extra_substituters: Option<Vec<String>>,
}

/// Read Nix cache URL from flake.nix using `nix eval`.
/// Returns the first extra-substituter if available.
pub fn read_cache_url() -> Result<Option<String>> {
    info!("Reading Nix config for cache URL");

    let output = std::process::Command::new("nix")
        .args([
            "eval",
            "-f",
            "flake.nix",
            "nixConfig.extra-substituters",
            "--json",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!("Failed to read Nix config: {}", stderr);
        return Ok(None);
    }

    let config: NixConfig = serde_json::from_slice(&output.stdout)?;

    let url = config
        .extra_substituters
        .and_then(|mut v| v.pop())
        .filter(|url| !url.is_empty());

    if let Some(ref url) = url {
        info!("Found cache URL: {}", url);
    } else {
        warn!("No cache URL found in Nix config");
    }

    Ok(url)
}

/// Check if Nix artifacts are available in current environment.
/// Looks for typical Nix derivation outputs (result symlinks or store paths).
pub fn find_nix_artifacts() -> Vec<String> {
    let mut artifacts = Vec::new();

    // Check for ./result symlink (nix-build output)
    if std::path::Path::new("result").exists() {
        if let Ok(path) = std::fs::read_link("result") {
            if let Some(path_str) = path.to_str() {
                artifacts.push(path_str.to_string());
            }
        }
    }

    // Check for result-* patterns
    if let Ok(entries) = std::fs::read_dir(".") {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("result-") && entry.path().is_file() {
                    artifacts.push(name.to_string());
                }
            }
        }
    }

    artifacts
}

/// Push Nix derivation to cache.
/// Runs: nix copy --to <url> <path>
pub fn push_to_cache(url: &str, derivation_path: &str) -> Result<()> {
    info!("Pushing {} to cache {}", derivation_path, url);

    let status = std::process::Command::new("nix")
        .args(["copy", "--to", url, derivation_path])
        .status()?;

    if !status.success() {
        return Err(CacheError::NixCommandFailed {
            stderr: "nix copy failed".to_string(),
        });
    }

    info!("Successfully pushed to cache");
    Ok(())
}

/// Pull Nix derivation from cache.
/// Runs: nix copy --from <url> <path>
pub fn pull_from_cache(url: &str, derivation_path: &str) -> Result<()> {
    info!("Pulling {} from cache {}", derivation_path, url);

    let status = std::process::Command::new("nix")
        .args(["copy", "--from", url, derivation_path])
        .status()?;

    if !status.success() {
        return Err(CacheError::NixCommandFailed {
            stderr: "nix copy failed".to_string(),
        });
    }

    info!("Successfully pulled from cache");
    Ok(())
}

/// Push all available Nix artifacts to cache.
pub fn push_all_to_cache(url: &str) -> Result<()> {
    let artifacts = find_nix_artifacts();

    if artifacts.is_empty() {
        warn!("No Nix artifacts found to push");
        return Err(CacheError::NoArtifacts);
    }

    info!("Pushing {} artifacts to cache", artifacts.len());

    for artifact in &artifacts {
        if let Err(e) = push_to_cache(url, artifact) {
            error!("Failed to push {}: {}", artifact, e);
            // Continue with other artifacts
        }
    }

    Ok(())
}

=======
/// Result type for cache operations.
pub type Result<T> = std::result::Result<T, CacheError>;

/// Read the configured Nix cache URL from the flake at `repo_path`.
///
/// Reads `nixConfig.extra-substituters` (or `nixConfig.substituters` as fallback)
/// from the flake definition and returns the first URL found.
///
/// Returns `Ok(None)` when the flake has no cache configured (not an error).
pub fn read_cache_url(repo_path: &Path) -> Result<Option<String>> {
    // Try extra-substituters first (idiomatic for flake-specific caches)
    if let Some(url) = try_read_substituters(repo_path, "extra-substituters")? {
        return Ok(Some(url));
    }

    // Fall back to substituters
    if let Some(url) = try_read_substituters(repo_path, "substituters")? {
        return Ok(Some(url));
    }

    Ok(None)
}

/// Run `nix eval -f flake.nix nixConfig.<attr> --json` and return the first URL.
fn try_read_substituters(repo_path: &Path, attr: &str) -> Result<Option<String>> {
    let flake_path = repo_path.join("flake.nix");
    if !flake_path.exists() {
        return Ok(None);
    }

    let output = Command::new("nix")
        .arg("eval")
        .arg("-f")
        .arg(&flake_path)
        .arg(format!("nixConfig.{}", attr))
        .arg("--json")
        .output()?;

    if !output.status.success() {
        // Attribute not found or eval error — not configured
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let urls: Vec<String> = serde_json::from_str(&stdout)?;

    Ok(urls.into_iter().find(|u| !u.is_empty()))
}

/// Push all locally-built Nix artifacts to a binary cache.
///
/// Discovers `./result` symlinks (including `result-*`) created by
/// `nix build` and copies each to the cache via `nix copy --to <url>`.
pub fn push_all_to_cache(url: &str) -> Result<()> {
    let artifacts = find_nix_artifacts()?;

    if artifacts.is_empty() {
        return Err(CacheError::NoArtifacts);
    }

    let mut last_err = None;

    for artifact in &artifacts {
        warn!("Pushing {} to cache {}", artifact.display(), url);
        if let Err(e) = push_to_cache(url, artifact) {
            warn!("Failed to push {}: {}", artifact.display(), e);
            last_err = Some(e);
        }
    }

    match last_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Push a single store path (or symlink to one) to the cache.
pub fn push_to_cache(url: &str, path: &Path) -> Result<()> {
    let output = Command::new("nix")
        .arg("copy")
        .arg("--to")
        .arg(url)
        .arg(path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(CacheError::NixCommandFailed {
            exit_code: output.status.code(),
            stderr,
        });
    }

    Ok(())
}

/// Pull a single store path from the cache.
pub fn pull_from_cache(url: &str, path: &Path) -> Result<()> {
    let output = Command::new("nix")
        .arg("copy")
        .arg("--from")
        .arg(url)
        .arg(path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(CacheError::NixCommandFailed {
            exit_code: output.status.code(),
            stderr,
        });
    }

    Ok(())
}

/// Find Nix build artifacts in the current directory.
///
/// Looks for `./result` and `./result-*` symlinks (created by `nix build`).
pub fn find_nix_artifacts() -> Result<Vec<std::path::PathBuf>> {
    let mut artifacts = Vec::new();
    let cwd = std::env::current_dir()?;

    // Check for ./result
    let result_path = cwd.join("result");
    if result_path.exists() {
        artifacts.push(result_path);
    }

    // Check for ./result-*
    if let Ok(entries) = std::fs::read_dir(&cwd) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("result-") && entry.path().exists() {
                artifacts.push(entry.path());
            }
        }
    }

    Ok(artifacts)
}

>>>>>>> master
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
<<<<<<< HEAD
    fn test_find_nix_artifacts_empty() {
        // In a temp directory with no artifacts
        let artifacts = find_nix_artifacts();
        // May find nothing or may find something depending on environment
        // Just verify it doesn't panic
        let _ = artifacts;
=======
    fn test_error_display() {
        let err = CacheError::NoCacheConfig;
        assert_eq!(
            format!("{}", err),
            "No cache URL configured in flake nixConfig"
        );

        let err = CacheError::NoArtifacts;
        assert_eq!(
            format!("{}", err),
            "No Nix build artifacts found (no ./result)"
        );

        let err = CacheError::NixCommandFailed {
            exit_code: Some(1),
            stderr: "something went wrong".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("something went wrong"));
>>>>>>> master
    }
}
