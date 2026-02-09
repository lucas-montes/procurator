use std::collections::HashMap;

use askama::Template;
use serde::Serialize;

use crate::{
    mapping::{Language, PackageManager, Version},
    repo::analysis::{Analysis, Metadata, RepoAnalysis, Service, Toolchain},
};

/// Nix flake configuration
/// Maps one-to-one with a real flake.nix structure
#[derive(Debug, Serialize, Template)]
#[template(path = "flake.jinja")]
pub struct Configuration<'write> {
    /// Flake description
    description: String,

    /// Flake inputs (nixpkgs, etc.)
    inputs: Inputs<'write>,

    /// Flake outputs
    outputs: Outputs,
}


impl<'write> From<Analysis> for Configuration<'write> {
    fn from(analysis: Analysis) -> Self {
        // Filter out empty RepoAnalysis (no packages, no checks, no meaningful data)
        let all_repos = analysis.repos();
        let non_empty_repos: Vec<&RepoAnalysis> = all_repos
            .iter()
            .filter(|repo| !repo.packages().is_empty() || !repo.checks().iter().next().is_some())
            .collect();

        // Collect all packages from all repos
        let packages = collect_packages(&non_empty_repos);

        // Merge dev_tools into single devShell
        let dev_shell = merge_dev_shells(&non_empty_repos);

        // Collect and namespace checks (handle duplicates)
        let checks = collect_checks(&non_empty_repos);

        // Generate description from repo names
        let repo_names: Vec<_> = non_empty_repos.iter().map(|r| r.name()).collect();
        let description = if repo_names.is_empty() {
            "Auto-generated flake".to_string()
        } else {
            format!("Auto-generated flake for {}", repo_names.join(", "))
        };

        // Extract procurator extensions
        let procurator = extract_extensions(&non_empty_repos);

        Self {
            description,
            inputs: Inputs::default(),
            outputs: Outputs {
                packages,
                dev_shells: dev_shell,
                checks,
                procurator,
            },
        }
    }
}

/// Helper functions for extraction
impl<'write> Configuration<'write> {
    pub fn to_nix(&self) -> Result<String, askama::Error> {
        self.render()
    }
}

/// Flake inputs
#[derive(Debug, Serialize)]
struct Inputs<'write> {
    nixpkgs: &'write str,
}

impl<'write> Default for Inputs<'write> {
    fn default() -> Self {
        Self {
            nixpkgs: "github:NixOS/nixpkgs/nixos-unstable",
        }
    }
}

/// Flake outputs
#[derive(Debug, Serialize)]
struct Outputs {
    /// Packages per system
    packages: HashMap<String, PackageOutput>,

    /// Dev shells per system (merged from all repos)
    dev_shells: DevShellOutput,

    /// Checks per system
    checks: HashMap<String, CheckOutput>,

    /// Custom procurator extensions (service orchestration, project metadata)
    procurator: ProcuratorExtensions,
}

/// Package output configuration
#[derive(Debug, Serialize)]
struct PackageOutput {
    name: String,
    toolchain: ToolchainConfig,
    dependencies: Vec<String>, // nixpkgs package names
    metadata: MetadataConfig,
}

/// Toolchain configuration for building packages
#[derive(Debug, Clone, Serialize)]
struct ToolchainConfig {
    language: Language,
    package_manager: PackageManager,
    version: Version,
}

impl From<&Toolchain> for ToolchainConfig {
    fn from(tc: &Toolchain) -> Self {
        Self {
            language: tc.language().clone(),
            package_manager: tc.package_manager().clone(),
            version: tc.version().clone(),
        }
    }
}

/// Development shell configuration
#[derive(Debug, Serialize)]
struct DevShellOutput {
    /// All unique toolchains from all packages
    toolchains: Vec<ToolchainConfig>,

    /// All unique dependencies
    dependencies: Vec<String>,

    /// Merged environment variables
    env: HashMap<String, String>,

    /// Shell hook
    shell_hook: Option<String>,

    /// Services available in dev shell
    services: Vec<ServiceConfig>,
}

impl DevShellOutput {
    pub fn shell_hook_str(&self) -> &str {
        self.shell_hook.as_deref().unwrap_or("")
    }
}

/// Check output configuration
#[derive(Debug, Serialize)]
struct CheckOutput {
    name: String,
    command: String,
    toolchain: ToolchainConfig,
    dependencies: Vec<String>,
    services: Vec<ServiceConfig>,
}

/// Service configuration
#[derive(Debug, Clone, Serialize)]
pub struct ServiceConfig {
    name: String,
    version: Option<String>,
    port: Option<u16>,
    env: HashMap<String, String>,
}

impl From<&Service> for ServiceConfig {
    fn from(service: &Service) -> Self {
        Self {
            name: service.name().to_string(),
            version: Some(service.version().to_nix()),
            port: infer_default_port(service.name()),
            env: HashMap::new(),
        }
    }
}

/// Package metadata
#[derive(Debug, Clone, Serialize)]
struct MetadataConfig {
    version: String,
    description: Option<String>,
    authors: Vec<String>,
    license: Option<String>,
}

impl MetadataConfig {
    pub fn description_str(&self) -> &str {
        self.description.as_deref().unwrap_or("")
    }

    pub fn license_str(&self) -> &str {
        self.license.as_deref().unwrap_or("")
    }
}

impl From<&Metadata> for MetadataConfig {
    fn from(meta: &Metadata) -> Self {
        Self {
            version: meta.version().to_nix(),
            description: meta.description().clone(),
            authors: meta.authors().clone(),
            license: meta.license().clone(),
        }
    }
}

/// Custom procurator extensions
/// These go in the flake's `procurator` output attribute
#[derive(Debug, Serialize)]
struct ProcuratorExtensions {
    /// All services with full metadata
    services: Vec<ServiceDefinition>,

    /// Project metadata
    project: ProjectMetadata,
}

/// Full service definition with orchestration metadata
#[derive(Debug, Serialize)]
struct ServiceDefinition {
    name: String,
    version: Option<String>,
    port: Option<u16>,
    health_check: Option<String>,
    depends_on: Vec<String>,
    env_vars: HashMap<String, String>,
}

impl ServiceDefinition {
    pub fn version_str(&self) -> &str {
        self.version.as_deref().unwrap_or("latest")
    }

    pub fn health_check_str(&self) -> &str {
        self.health_check.as_deref().unwrap_or("")
    }

    pub fn has_port(&self) -> bool {
        self.port.is_some()
    }

    pub fn port_value(&self) -> u16 {
        self.port.unwrap_or(0)
    }
}

/// Project metadata
#[derive(Debug, Serialize)]
struct ProjectMetadata {

    name: String,
    languages: Vec<Language>,
    package_managers: Vec<PackageManager>,
}


fn collect_packages(repos: &[&RepoAnalysis]) -> HashMap<String, PackageOutput> {
    let mut packages = HashMap::new();

    for repo in repos {
        for package in repo.packages().iter() {
            let pkg_output = PackageOutput {
                name: package.name().to_string(),
                toolchain: ToolchainConfig::from(package.toolchain()),
                dependencies: extract_dep_names(package.dependencies()),
                metadata: MetadataConfig::from(package.metadata()),
            };
            packages.insert(package.name().to_string(), pkg_output);
        }
    }

    packages
}

fn merge_dev_shells(repos: &[&RepoAnalysis]) -> DevShellOutput {
    use std::collections::HashSet;

    let mut toolchains_set = HashSet::new();
    let mut dependencies_set = HashSet::new();
    let mut env = HashMap::new();
    let mut services_set = HashSet::new();

    for repo in repos {
        // Collect unique toolchains from packages
        for package in repo.packages().iter() {
            toolchains_set.insert(format!(
                "{:?}-{:?}",
                package.toolchain().language(),
                package.toolchain().version().to_nix(),
            ));
        }

        // Collect dependencies from dev_tools
        for dep in extract_dep_names(repo.dev_tools().dependencies()) {
            dependencies_set.insert(dep);
        }

        // Merge environment variables (last one wins, with warning comment)
        for (key, value) in repo.dev_tools().env() {
            if let Some(existing) = env.insert(key.clone(), value.clone()) {
                if existing != *value {
                    eprintln!(
                        "Warning: Environment variable {} has conflicting values: '{}' vs '{}'",
                        key, existing, value
                    );
                }
            }
        }

        // Collect unique services
        for service in repo.dev_tools().services().iter() {
            services_set.insert(service.name().to_string());
        }
    }

    // Reconstruct toolchains from unique set
    let mut toolchains = Vec::new();
    for repo in repos {
        for package in repo.packages().iter() {
            let key = format!(
                "{:?}-{:?}",
                package.toolchain().language(),
                package.toolchain().version().to_nix()
            );
            if toolchains_set.remove(&key) {
                toolchains.push(ToolchainConfig::from(package.toolchain()));
            }
        }
    }

    // Collect service configs
    let mut services = Vec::new();
    for repo in repos {
        for service in repo.dev_tools().services().iter() {
            if services_set.remove(service.name()) {
                services.push(ServiceConfig::from(service));
            }
        }
    }

    DevShellOutput {
        toolchains,
        dependencies: dependencies_set.into_iter().collect(),
        env,
        shell_hook: None,
        services,
    }
}

fn collect_checks(repos: &[&RepoAnalysis]) -> HashMap<String, CheckOutput> {
    let mut checks = HashMap::new();
    let mut name_counts: HashMap<String, usize> = HashMap::new();

    for repo in repos {
        for check in repo.checks().iter() {
            // Track duplicate names
            let count = name_counts.entry(check.name().to_string()).or_insert(0);
            *count += 1;

            // Namespace if duplicate
            let check_name = if *count > 1 {
                // Infer source from command
                let source = infer_check_source(check.command());
                format!("{}-{}", check.name(), source)
            } else {
                check.name().to_string()
            };

            let check_output = CheckOutput {
                name: check_name.clone(),
                command: check.command().to_string(),
                toolchain: ToolchainConfig::from(check.toolchain()),
                dependencies: extract_dep_names(check.dependencies()),
                services: check
                    .services()
                    .iter()
                    .map(ServiceConfig::from)
                    .collect(),
            };

            checks.insert(check_name, check_output);
        }
    }

    checks
}

fn extract_extensions(repos: &[&RepoAnalysis]) -> ProcuratorExtensions {
    use std::collections::HashSet;

    let mut services_map: HashMap<String, ServiceDefinition> = HashMap::new();
    let mut languages_set = HashSet::new();
    let mut package_managers_set = HashSet::new();

    // Collect services from all repos
    for repo in repos {
        // From dev_tools
        for service in repo.dev_tools().services().iter() {
            let service_def = ServiceDefinition {
                name: service.name().to_string(),
                version: Some(service.version().to_nix()),
                port: infer_default_port(service.name()),
                health_check: None, // TODO: Extract from docker-compose
                depends_on: Vec::new(), // TODO: Extract dependencies
                env_vars: HashMap::new(),
            };
            services_map.insert(service.name().to_string(), service_def);
        }

        // From checks
        for check in repo.checks().iter() {
            for service in check.services().iter() {
                services_map
                    .entry(service.name().to_string())
                    .or_insert_with(|| ServiceDefinition {
                        name: service.name().to_string(),
                        version: Some(service.version().to_nix()),
                        port: infer_default_port(service.name()),
                        health_check: None,
                        depends_on: Vec::new(),
                        env_vars: HashMap::new(),
                    });
            }
        }

        // Collect languages and package managers
        for package in repo.packages().iter() {
            languages_set.insert(package.toolchain().language().clone());
            package_managers_set.insert(package.toolchain().package_manager().clone());
        }
    }

    let project_name = repos
        .first()
        .map(|r| r.name().to_string())
        .unwrap_or_else(|| "project".to_string());

    ProcuratorExtensions {
        services: services_map.into_values().collect(),
        project: ProjectMetadata {
            name: project_name,
            languages: languages_set.into_iter().collect(),
            package_managers: package_managers_set.into_iter().collect(),
        },
    }
}

// Helper functions

fn extract_dep_names(dependencies: &crate::repo::analysis::Dependencies) -> Vec<String> {
    dependencies.iter().map(|d| d.name().to_string()).collect()
}

fn infer_check_source(command: &str) -> &str {
    if command.starts_with("make ") {
        "makefile"
    } else if command.contains("npm ") || command.contains("jest") || command.contains("prettier") {
        "npm-script"
    } else if command.contains("cargo ") {
        "cargo"
    } else {
        "script"
    }
}

fn infer_default_port(service_name: &str) -> Option<u16> {
    match service_name.to_lowercase().as_str() {
        "postgres" | "postgresql" => Some(5432),
        "redis" => Some(6379),
        "mysql" | "mariadb" => Some(3306),
        "mongodb" | "mongo" => Some(27017),
        "elasticsearch" => Some(9200),
        "rabbitmq" => Some(5672),
        "memcached" => Some(11211),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    fn fixtures_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("analysis")
    }

}
