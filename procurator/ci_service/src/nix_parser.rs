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
use std::process::Command;
use tracing::{error, info};

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

/// Parse flake metadata from a git repository
pub async fn get_flake_metadata(repo_path: &str) -> Result<FlakeMetadata> {
    info!(repo_path = repo_path, "Parsing flake metadata");

    // Use nix flake metadata command to get structured info
    let output = Command::new("nix")
        .args(&["flake", "metadata", "--json", &format!("git+file://{}", repo_path)])
        .output()
        .map_err(|e| NixParserError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!(repo_path = repo_path, stderr = stderr.as_ref(), "Failed to get flake metadata");
        return Err(NixParserError::NotAFlake);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let metadata: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| NixParserError::ParseError(e.to_string()))?;

    let description = metadata["description"]
        .as_str()
        .map(|s| s.to_string());

    // Now get flake show output for packages, checks, etc.
    let show_output = Command::new("nix")
        .args(&["flake", "show", "--json", &format!("git+file://{}", repo_path)])
        .output()
        .map_err(|e| NixParserError::CommandFailed(e.to_string()))?;

    let show_stdout = String::from_utf8_lossy(&show_output.stdout);
    let show_data: serde_json::Value = serde_json::from_str(&show_stdout)
        .unwrap_or(serde_json::json!({}));

    // Extract packages, checks, apps, etc.
    let mut packages = Vec::new();
    let mut checks = Vec::new();
    let mut apps = Vec::new();
    let mut dev_shells = Vec::new();
    let mut nixos_modules = Vec::new();

    // Parse the flake show output structure
    // Structure is: { "system": { "packages": { "name": {...} } } }
    if let Some(obj) = show_data.as_object() {
        for (system_name, system_data) in obj {
            // Skip metadata fields
            if system_name == "description" {
                continue;
            }

            if let Some(system_obj) = system_data.as_object() {
                // Packages
                if let Some(packages_obj) = system_obj.get("packages").and_then(|v| v.as_object()) {
                    for package_name in packages_obj.keys() {
                        packages.push(format!("{}.{}", system_name, package_name));
                    }
                }

                // Checks
                if let Some(checks_obj) = system_obj.get("checks").and_then(|v| v.as_object()) {
                    for check_name in checks_obj.keys() {
                        checks.push(format!("{}.{}", system_name, check_name));
                    }
                }

                // Apps
                if let Some(apps_obj) = system_obj.get("apps").and_then(|v| v.as_object()) {
                    for app_name in apps_obj.keys() {
                        apps.push(format!("{}.{}", system_name, app_name));
                    }
                }

                // Dev shells
                if let Some(shells_obj) = system_obj.get("devShells").and_then(|v| v.as_object()) {
                    for shell_name in shells_obj.keys() {
                        dev_shells.push(format!("{}.{}", system_name, shell_name));
                    }
                }
            }
        }

        // NixOS modules (not system-specific)
        if let Some(modules_obj) = obj.get("nixosModules").and_then(|v| v.as_object()) {
            for module_name in modules_obj.keys() {
                nixos_modules.push(module_name.clone());
            }
        }
    }

    Ok(FlakeMetadata {
        description,
        packages,
        checks,
        apps,
        dev_shells,
        nixos_modules,
    })
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
