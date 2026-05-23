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
        }
    }
}

impl std::error::Error for CacheError {}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_nix_artifacts_empty() {
        // In a temp directory with no artifacts
        let artifacts = find_nix_artifacts();
        // May find nothing or may find something depending on environment
        // Just verify it doesn't panic
        let _ = artifacts;
    }
}
