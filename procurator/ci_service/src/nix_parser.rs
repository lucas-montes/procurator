//! Nix Flake Parser
//!
//! Extracts metadata from Nix flakes to understand available outputs.
//! Currently parses:
//! - Packages
//! - Checks (tests)
//! - Apps
//! - Dev shells
//! - NixOS modules
//!
//! This is a foundation for future work to parse `procurator.*` flake outputs
//! that describe CI jobs, deployments, and other configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use tracing::{error, info};

// ============================================================================
// Nix Command Output Structures
// ============================================================================

/// Output structure from `nix flake metadata --json`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NixFlakeMetadataOutput {
    pub description: Option<String>,
    pub path: Option<String>,
    pub url: Option<String>,
    pub original_url: Option<String>,
    pub resolved_url: Option<String>,
    pub last_modified: Option<i64>,
    pub fingerprint: Option<String>,
    pub dirty_revision: Option<String>,
    pub locks: Option<NixFlakeLocks>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NixFlakeLocks {
    pub version: Option<i64>,
    pub root: Option<String>,
    pub nodes: Option<HashMap<String, serde_json::Value>>,
}

/// Output structure from `nix flake show --json`
#[derive(Debug, Clone, Deserialize, Default)]
pub struct NixFlakeShowOutput {
    #[serde(default)]
    pub packages: HashMap<String, HashMap<String, NixDerivationInfo>>,
    #[serde(default)]
    pub checks: HashMap<String, HashMap<String, NixDerivationInfo>>,
    #[serde(default)]
    pub apps: HashMap<String, HashMap<String, NixAppInfo>>,
    #[serde(default, rename = "devShells")]
    pub dev_shells: HashMap<String, HashMap<String, NixDerivationInfo>>,
    #[serde(default, rename = "nixosModules")]
    pub nixos_modules: HashMap<String, NixModuleInfo>,
    #[serde(default, rename = "nixosConfigurations")]
    pub nixos_configurations: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct NixDerivationInfo {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "type")]
    pub derivation_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct NixAppInfo {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "type")]
    pub app_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct NixModuleInfo {
    #[serde(default, rename = "type")]
    pub module_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakeMetadata {
    pub description: Option<String>,
    pub packages: Vec<String>,
    pub checks: Vec<String>,
    pub apps: Vec<String>,
    pub dev_shells: Vec<String>,
    pub nixos_modules: Vec<String>,
}

#[derive(Debug)]
pub enum NixParserError {
    CommandFailed(String),
    ParseError(String),
    NotAFlake,
}

impl std::fmt::Display for NixParserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NixParserError::CommandFailed(msg) => write!(f, "Command failed: {}", msg),
            NixParserError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            NixParserError::NotAFlake => write!(f, "Not a Nix flake"),
        }
    }
}

impl std::error::Error for NixParserError {}

type Result<T> = std::result::Result<T, NixParserError>;


/// Run a nix command and return the parsed JSON output
fn run_nix_command<T: serde::de::DeserializeOwned>(args: &[&str]) -> Result<T> {
    let output = Command::new("nix")
        .args(args)
        .output()
        .map_err(|e| NixParserError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NixParserError::CommandFailed(stderr.to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).map_err(|e| NixParserError::ParseError(e.to_string()))
}

/// Parse flake metadata from a git repository
pub async fn get_flake_metadata(repo_path: &str) -> Result<FlakeMetadata> {
    let path = Path::new(repo_path);
    let flake_url = format!("git+file://{}", repo_path.display());

    info!(repo_path = repo_path, flake_url = %flake_url, "Parsing flake metadata");

    // Get flake metadata
    let metadata: NixFlakeMetadataOutput =
        run_nix_command(&["flake", "metadata", "--json", &flake_url]).map_err(|e| {
            error!(repo_path = repo_path, error = %e, "Failed to get flake metadata");
            NixParserError::NotAFlake
        })?;

    // Get flake show output for packages, checks, etc.
    let show_output: NixFlakeShowOutput =
        run_nix_command(&["flake", "show", "--json", &flake_url]).unwrap_or_default();

    // Extract outputs as flattened paths (system.name format)
    let packages = flatten_system_outputs(&show_output.packages);
    let checks = flatten_system_outputs(&show_output.checks);
    let apps = flatten_system_outputs(&show_output.apps);
    let dev_shells = flatten_system_outputs(&show_output.dev_shells);
    let nixos_modules: Vec<String> = show_output.nixos_modules.keys().cloned().collect();

    Ok(FlakeMetadata {
        description: metadata.description,
        packages,
        checks,
        apps,
        dev_shells,
        nixos_modules,
    })
}

/// Flatten system-specific outputs into "system.name" format
fn flatten_system_outputs<T>(outputs: &HashMap<String, HashMap<String, T>>) -> Vec<String> {
    outputs
        .iter()
        .flat_map(|(system, items)| {
            items
                .keys()
                .map(move |name| format!("{}.{}", system, name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires actual git repo
    async fn test_parse_flake() {
        let metadata = get_flake_metadata("/path/to/test/repo.git").await;
        assert!(metadata.is_ok());
    }
}
