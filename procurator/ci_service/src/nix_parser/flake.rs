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
use std::process::Command;
use tracing::{error, info};

use crate::repo_manager::RepoPath;

/// Output structure from `nix flake metadata --json`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NixFlakeMetadataOutput {
    description: Option<String>,
    path: Option<String>,
    url: Option<String>,
    original_url: Option<String>,
    resolved_url: Option<String>,
    last_modified: Option<i64>,
    fingerprint: Option<String>,
    dirty_revision: Option<String>,
    locks: Option<NixFlakeLocks>,
}

#[derive(Debug, Clone, Deserialize)]
struct NixFlakeLocks {
    version: Option<i64>,
    root: Option<String>,
    nodes: Option<HashMap<String, serde_json::Value>>,
}

/// Output structure from `nix flake show --json`
#[derive(Debug, Clone, Deserialize, Default)]
struct NixFlakeShowOutput {
    #[serde(default)]
    packages: HashMap<String, HashMap<String, NixDerivationInfo>>,
    #[serde(default)]
    checks: HashMap<String, HashMap<String, NixDerivationInfo>>,
    #[serde(default)]
    apps: HashMap<String, HashMap<String, NixAppInfo>>,
    #[serde(default, rename = "devShells")]
    dev_shells: HashMap<String, HashMap<String, NixDerivationInfo>>,
    #[serde(default, rename = "nixosModules")]
    nixos_modules: HashMap<String, NixModuleInfo>,
    #[serde(default, rename = "nixosConfigurations")]
    nixos_configurations: HashMap<String, serde_json::Value>,
    #[serde(default)]
    infrastructure: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct NixDerivationInfo {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "type")]
    derivation_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct NixAppInfo {
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "type")]
    app_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct NixModuleInfo {
    #[serde(default, rename = "type")]
    module_type: Option<String>,
}

// ============================================================================
// Infrastructure Types
// ============================================================================

/// Infrastructure configuration from the flake
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Infrastructure {
    #[serde(default)]
    pub machines: HashMap<String, Machine>,
    #[serde(default)]
    pub services: HashMap<String, Service>,
    #[serde(default)]
    pub cd: ContinuousDelivery,
    #[serde(default)]
    pub rollback: RollbackConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Machine {
    pub cpu: f64,
    pub memory: MemorySpec,
    #[serde(default)]
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySpec {
    pub amount: f64,
    #[serde(default = "default_unit")]
    pub unit: String,
}

fn default_unit() -> String {
    "GB".to_string()
}

/// Service definition - supports multiple source types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    #[serde(rename = "sourceInfo")]
    pub source_info: ServiceSourceInfo,
    #[serde(default)]
    pub environments: HashMap<String, Environment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSourceInfo {
    #[serde(rename = "type")]
    pub source_type: String, // "package", "flake", or "url"
    pub url: Option<String>,
    pub rev: Option<String>,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    pub cpu: f64,
    pub memory: MemorySpec,
    #[serde(default = "default_replicas")]
    pub replicas: i32,
    #[serde(rename = "healthCheck")]
    pub health_check: Option<String>,
}

fn default_replicas() -> i32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContinuousDelivery {
    #[serde(default)]
    pub tests: bool,
    #[serde(default)]
    pub build: bool,
    #[serde(default)]
    pub dst: bool,
    #[serde(default)]
    pub staging: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RollbackConfig {
    #[serde(default)]
    pub enabled: bool,
    pub threshold: Option<RollbackThreshold>,
    pub notification: Option<NotificationConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackThreshold {
    pub cpu: Option<f64>,
    pub memory: Option<MemorySpec>,
    pub latency: Option<LatencyThreshold>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyThreshold {
    pub p99: Option<String>,
    pub p90: Option<String>,
    pub p50: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    pub enabled: bool,
    pub email: Option<EmailConfig>,
    pub slack: Option<SlackConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    pub subject: String,
    pub body: String,
    pub recipients: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    pub channel: String,
    pub message: String,
    #[serde(rename = "webhookUrl")]
    pub webhook_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakeMetadata {
    pub description: Option<String>,
    pub packages: Vec<String>,
    pub checks: Vec<String>,
    pub apps: Vec<String>,
    pub dev_shells: Vec<String>,
    pub nixos_modules: Vec<String>,
    pub infrastructure: Option<Infrastructure>,
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
    serde_json::from_slice(&output.stdout).map_err(|e| NixParserError::ParseError(e.to_string()))
}

/// Get the HEAD commit hash from a bare git repository
fn get_head_rev(bare_repo_path: &std::path::Path) -> Result<String> {
    let output = Command::new("git")
        .args(["--git-dir", &bare_repo_path.to_string_lossy()])
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| NixParserError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        return Err(NixParserError::CommandFailed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Parse infrastructure configuration from flake output
fn parse_infrastructure(flake_url: &str) -> Option<Infrastructure> {
    let flake_url = format!("{}#infrastructure", flake_url);
    info!(flake_url = %flake_url, "Parsing infrastructure configuration from flake");
    let output = match Command::new("nix")
        .args(["eval", "--json", &flake_url])
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            error!(flake_url = %flake_url, error = %e, "Failed to run nix eval for infrastructure");
            return None;
        }
    };

    if !output.status.success() {
        tracing::debug!("No infrastructure output found in flake");
        return None;
    }

    info!(output = ?str::from_utf8(&output.stdout), "Parsing infrastructure configuration from flake");

    match serde_json::from_slice(&output.stdout) {
        Ok(infra) => {
            tracing::info!("Successfully parsed infrastructure from flake output");
            Some(infra)
        }
        Err(e) => {
            tracing::warn!("Failed to deserialize infrastructure: {}", e);
            None
        }
    }
}

/// Parse infrastructure configuration directly from a repository
pub fn parse_infrastructure_from_repo(repo_path: &RepoPath) -> Option<Infrastructure> {
    let bare_path = repo_path.bare_repo_path();
    let head_rev = get_head_rev(&bare_path).ok()?;
    let flake_url = repo_path.to_nix_url_with_rev(&head_rev);

    info!(bare_path = %bare_path.display(), repo_path = %repo_path, flake_url = %flake_url, "Parsing infrastructure configuration");

    parse_infrastructure(&flake_url)
}

impl TryFrom<&RepoPath> for FlakeMetadata {
    type Error = NixParserError;

    fn try_from(repo_path: &RepoPath) -> Result<Self> {
        // For bare repos, we need to specify a revision
        let bare_path = repo_path.bare_repo_path();
        let head_rev = get_head_rev(&bare_path)?;
        let flake_url = repo_path.to_nix_url_with_rev(&head_rev);

        info!(repo_path = %repo_path, flake_url = %flake_url, "Parsing flake metadata");

        // Get flake metadata
        let metadata: NixFlakeMetadataOutput =
            run_nix_command(&["flake", "metadata", "--json", &flake_url]).map_err(|e| {
                error!(repo_path = %repo_path, error = %e, "Failed to get flake metadata");
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

        // Try to parse infrastructure configuration (system-independent top-level output)
        let infrastructure = parse_infrastructure(&flake_url);

        Ok(Self {
            description: metadata.description,
            packages,
            checks,
            apps,
            dev_shells,
            nixos_modules,
            infrastructure,
        })
    }
}

/// Flatten system-specific outputs into "system.name" format
fn flatten_system_outputs<T>(outputs: &HashMap<String, HashMap<String, T>>) -> Vec<String> {
    outputs
        .iter()
        .flat_map(|(system, items)| items.keys().map(move |name| format!("{}.{}", system, name)))
        .collect()
}
