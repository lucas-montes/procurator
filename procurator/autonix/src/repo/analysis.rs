use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use crate::{
    mapping::{
        Language, PackageManager, ParsedCiCdFile, ParsedContainerFile, ParsedManifest,
        ParsedTaskFile, Version,
    },
    repo::scan::{Repo, ScanIter},
};

/// Analysis result for the entire repository
/// Complete configuration for each repo in the repository
/// This is what we'll use to generate flake.nix
#[derive(Debug, PartialEq)]
pub struct Analysis(Vec<RepoAnalysis>);

impl From<ScanIter> for Analysis {
    fn from(scan: ScanIter) -> Self {
        Self(Vec::from_iter(scan.map(RepoAnalysis::from)))
    }
}

/// Complete configuration for a single repo
///
/// A repo can contain multiple packages (e.g., Rust workspace, npm workspaces)
/// This is the key intermediate representation that contains everything needed
/// to generate Nix expressions
#[derive(Debug, PartialEq)]
pub struct RepoAnalysis {
    /// Human-readable repo name
    name: String,

    /// Path from repository root to this repo
    path: PathBuf,

    /// Packages produced by this repo
    /// One repo can have multiple packages (workspace members, monorepo apps)
    packages: Packages,

    /// Development environment configuration
    dev_tools: DevTools,

    /// Check operations (tests, lints, formatting, etc.)
    checks: Checks,
}

impl From<Repo> for RepoAnalysis {
    fn from(repo: Repo) -> Self {
        let path = repo.path().to_path_buf();
        //NOTE: not nice, we possibly can avoid to many copy
        let name = path
            .file_name()
            .expect("the dir should have a name")
            .to_str()
            .expect("we should have hte str our the name")
            .to_owned();

        let ctx = ExtractionContext::from(&repo);

        let packages = Packages::from(&ctx);
        let dev_tools = DevTools::from(&ctx);
        let checks = Checks::from(&ctx);

        RepoAnalysis {
            name,
            path,
            packages,
            dev_tools,
            checks,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Packages(Vec<Package>);

impl From<&ExtractionContext<'_>> for Packages {
    fn from(ctx: &ExtractionContext<'_>) -> Self {
        let mut packages = Vec::new();
        let dependencies = extract_dependencies(ctx);

        // Create one package per manifest
        for manifest in &ctx.manifests {
            let language = Language::from(&manifest.manifest_type);
            let package_manager = PackageManager::from(&manifest.manifest_type);

            // Each name in the manifest becomes a package
            // (for workspaces with members, we skip them per requirement)
            for name in &manifest.names {
                let package = Package {
                    name: name.clone(),
                    path: ctx.repo.path().to_path_buf(), // Relative path from repo root
                    toolchain: Toolchain {
                        language,
                        package_manager,
                        version: manifest.toolchain_version.as_deref().into(),
                    },
                    dependencies: dependencies.clone(),
                    metadata: Metadata {
                        version: manifest.version.as_deref().into(),
                        description: manifest.metadata.description.clone(),
                        authors: manifest.metadata.authors.clone(),
                        license: manifest.metadata.license.clone(),
                    },
                };

                packages.push(package);
            }
        }

        Self(packages)
    }
}

/// A package that can be built from this repo
/// Examples: a binary crate in Cargo workspace, an app in npm workspace
#[derive(Debug, PartialEq)]
pub struct Package {
    /// Package name
    name: String,

    /// Path relative to repo root
    path: PathBuf,

    toolchain: Toolchain,

    /// Dependencies specific to this package
    dependencies: Dependencies,

    /// Package metadata (version, description, license, etc.)
    metadata: Metadata,
}

/// Toolchain configuration for the repo
#[derive(Debug, Clone, PartialEq)]
pub struct Toolchain {
    /// The main language (Rust, Python, TypeScript, etc.)
    language: Language,

    /// Primary package manager
    package_manager: PackageManager,

    /// Version constraint from manifest (e.g., "1.75.0", ">=3.11", "20.x")
    version: Version,
}

/// System packages from nixpkgs
/// Examples: pkgs.openssl, pkgs.postgresql, pkgs.pkg-config
/// Maps to buildInputs/nativeBuildInputs in Nix
#[derive(Debug, Clone, PartialEq)]
pub struct Dependencies(Vec<Dependency>);

/// A dependency can be either a running service or a build-time package
#[derive(Debug, Clone, PartialEq)]
pub struct Dependency {
    name: String,
    version: Version,
}

#[derive(Debug, PartialEq)]
pub struct Tool {
    name: String,
    version: Version,
}

/// Development tools configuration
/// Maps to devShells.<system>.default in flake.nix
#[derive(Debug, PartialEq)]
pub struct DevTools {
    /// Development tools (rust-analyzer, prettier, eslint, etc.)
    /// These are in addition to the base toolchain
    tools: Vec<Tool>,

    /// Environment variables for development
    env: HashMap<String, String>,

    /// Shell hook - runs when entering `nix develop`
    shell_hook: Option<String>,

    dependencies: Dependencies,
    services: Services,
}

impl From<&ExtractionContext<'_>> for DevTools {
    fn from(ctx: &ExtractionContext<'_>) -> Self {
        let dependencies = extract_dependencies(ctx);
        let services = extract_services(ctx);
        let env = extract_environment(ctx);

        // Extract tools from manifest scripts
        let mut all_scripts = HashMap::new();
        for manifest in &ctx.manifests {
            all_scripts.extend(manifest.scripts.clone());
        }
        let tools = infer_tools_from_scripts(&all_scripts);

        // Shell hook could be extracted from container entrypoint if needed
        let shell_hook = None;

        Self {
            tools,
            env,
            shell_hook,
            dependencies,
            services,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Checks(Vec<Check>);

impl From<&ExtractionContext<'_>> for Checks {
    fn from(ctx: &ExtractionContext<'_>) -> Self {
        let mut checks = Vec::new();
        let dependencies = extract_dependencies(ctx);
        let services = extract_services(ctx);

        // From CI/CD jobs - each job with steps becomes a check
        for cicd_file in &ctx.cicd {
            for job in &cicd_file.jobs {
                // If job has multiple unrelated steps, create separate checks
                // For now, combine steps with the same context (test, lint, build, etc.)
                let job_name = &job.name;

                // Group steps by inferred check type
                let mut test_commands = Vec::new();
                let mut lint_commands = Vec::new();
                let mut build_commands = Vec::new();
                let mut other_commands = Vec::new();

                for step in &job.steps {
                    if let Some(run_cmd) = &step.run {
                        let lower = run_cmd.to_lowercase();
                        if lower.contains("test") || lower.contains("jest") || lower.contains("pytest") {
                            test_commands.push(run_cmd.clone());
                        } else if lower.contains("lint") || lower.contains("eslint") || lower.contains("clippy") {
                            lint_commands.push(run_cmd.clone());
                        } else if lower.contains("build") || lower.contains("compile") {
                            build_commands.push(run_cmd.clone());
                        } else {
                            other_commands.push(run_cmd.clone());
                        }
                    }
                }

                // Create checks for each category
                if !test_commands.is_empty() {
                    checks.push(create_check(
                        format!("{}-test", job_name),
                        test_commands.join(" && "),
                        ctx,
                        &dependencies,
                        &services,
                    ));
                }
                if !lint_commands.is_empty() {
                    checks.push(create_check(
                        format!("{}-lint", job_name),
                        lint_commands.join(" && "),
                        ctx,
                        &dependencies,
                        &services,
                    ));
                }
                if !build_commands.is_empty() {
                    checks.push(create_check(
                        format!("{}-build", job_name),
                        build_commands.join(" && "),
                        ctx,
                        &dependencies,
                        &services,
                    ));
                }
                if !other_commands.is_empty() {
                    checks.push(create_check(
                        job_name.clone(),
                        other_commands.join(" && "),
                        ctx,
                        &dependencies,
                        &services,
                    ));
                }
            }
        }

        // From task file targets (Makefile, etc.)
        for task_file in &ctx.task_files {
            for target in &task_file.targets {
                // Common check targets
                if matches!(
                    target.as_str(),
                    "test" | "check" | "lint" | "format" | "fmt"
                ) {
                    checks.push(create_check(
                        target.clone(),
                        format!("make {}", target),
                        ctx,
                        &dependencies,
                        &services,
                    ));
                }
            }
        }

        // From manifest scripts
        for manifest in &ctx.manifests {
            for (script_name, script_cmd) in &manifest.scripts {
                // Common check script names
                if matches!(
                    script_name.as_str(),
                    "test" | "lint" | "format" | "check" | "typecheck"
                ) {
                    checks.push(create_check(
                        script_name.clone(),
                        script_cmd.clone(),
                        ctx,
                        &dependencies,
                        &services,
                    ));
                }
            }
        }

        Self(checks)
    }
}

/// Helper to create a Check with inferred toolchain
fn create_check(
    name: String,
    command: String,
    ctx: &ExtractionContext<'_>,
    dependencies: &Dependencies,
    services: &Services,
) -> Check {
    // Infer toolchain from the first manifest (primary language)
    let toolchain = if let Some(manifest) = ctx.manifests.first() {
        Toolchain {
            language: Language::from(&manifest.manifest_type),
            package_manager: PackageManager::from(&manifest.manifest_type),
            version: manifest.toolchain_version.as_deref().into(),
        }
    } else {
        // Default toolchain
        Toolchain {
            language: Language::Bash,
            package_manager: PackageManager::Pip, // TODO: wtf is this thing? why pip? I probably should rethink this thing here
            version: Version::default(),
        }
    };

    Check {
        name,
        command,
        toolchain,
        dependencies: dependencies.clone(),
        services: services.clone(),
    }
}

/// A check operation (test, lint, format check, etc.)
/// Maps to checks.<system>.<name> in flake.nix
#[derive(Debug, PartialEq)]
pub struct Check {
    /// Check name (e.g., "test", "lint", "format")
    name: String,

    /// Command to run (e.g., "cargo test", "eslint .")
    command: String,

    /// toolchain for this specific check
    toolchain: Toolchain,

    dependencies: Dependencies,
    services: Services,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Services(Vec<Service>);

/// A service dependency (database, cache, message queue, etc.)
#[derive(Debug, Clone, PartialEq)]
pub struct Service {
    /// Service name (postgresql, redis, mongodb, etc.)
    name: String,

    /// Version constraint if any
    version: Version,

    /// Configuration/setup needed to start the service
    /// TODO: maybe a Path would be better
    config: Option<String>,
}

/// Metadata from manifest files
/// Maps to meta.* in Nix derivation
#[derive(Debug, Clone, PartialEq)]
pub struct Metadata {
    /// Package version
    version: Version,

    /// Description
    description: Option<String>,

    /// Authors/maintainers
    authors: Vec<String>,

    /// License
    license: Option<String>,
}

/// Context for extraction containing all parsed files
struct ExtractionContext<'a> {
    repo: &'a Repo,
    manifests: Vec<ParsedManifest>,
    task_files: Vec<ParsedTaskFile>,
    cicd: Vec<ParsedCiCdFile>,
    containers: Vec<ParsedContainerFile>,
}

// 'ec for extraction context lifetime
impl<'ec: 'a, 'a> From<&'ec Repo> for ExtractionContext<'a> {
    fn from(repo: &'ec Repo) -> Self {
        // Parse all file types
        let parsed_manifests = repo
            .manifest_files()
            .iter()
            .filter_map(|file_path| file_path.parse().ok())
            .collect();

        let parsed_task_files = repo
            .task_files()
            .iter()
            .filter_map(|f| f.parse().ok())
            .collect();

        let parsed_cicd = repo
            .cicd_files()
            .iter()
            .filter_map(|f| f.parse().ok())
            .collect();

        let parsed_containers = repo
            .container_files()
            .iter()
            .filter_map(|f| f.parse().ok())
            .collect();

        // Build extraction context
        Self {
            repo: &repo,
            manifests: parsed_manifests,
            task_files: parsed_task_files,
            cicd: parsed_cicd,
            containers: parsed_containers,
        }
    }
}

/// Helper functions for extraction

/// Extract all system dependencies from context (deduplicated)
fn extract_dependencies(ctx: &ExtractionContext<'_>) -> Dependencies {
    let mut deps = HashSet::new();

    // From task files (Makefile, CMake, etc.)
    for task_file in &ctx.task_files {
        for dep in &task_file.system_deps {
            deps.insert(dep.clone());
        }
    }

    // From container files (Dockerfile RUN commands, docker-compose)
    for container in &ctx.containers {
        for pkg in &container.system_packages {
            deps.insert(pkg.clone());
        }
    }

    Dependencies(
        deps.into_iter()
            .map(|name| Dependency {
                name,
                version: Version::default(),
            })
            .collect(),
    )
}

/// Extract all services from context (deduplicated)
fn extract_services(ctx: &ExtractionContext<'_>) -> Services {
    let mut services_map: HashMap<String, Service> = HashMap::new();

    // From CI/CD service definitions
    for cicd_file in &ctx.cicd {
        for job in &cicd_file.jobs {
            for ci_service in &job.services {
                let (name, version) = parse_image_tag(&ci_service.image);
                services_map.entry(name.clone()).or_insert(Service {
                    name,
                    version,
                    config: None,
                });
            }
        }
    }

    // From container service definitions (docker-compose)
    for container in &ctx.containers {
        for container_service in &container.services {
            if let Some(image) = &container_service.image {
                let (name, version) = parse_image_tag(image);
                services_map.entry(name.clone()).or_insert(Service {
                    name,
                    version,
                    config: None,
                });
            }
        }
    }

    Services(services_map.into_values().collect())
}

/// Parse Docker image tag into service name and version
/// Example: "postgres:15" -> ("postgresql", Version("15"))
/// Example: "redis:7-alpine" -> ("redis", Version("7"))
fn parse_image_tag(image: &str) -> (String, Version) {
    let parts: Vec<&str> = image.split(':').collect();
    let name = parts[0].to_string();
    let version = if parts.len() > 1 {
        // Extract numeric version, ignore suffixes like "-alpine"
        let version_str = parts[1].split('-').next().unwrap_or(parts[1]);
        Version(Some(version_str.to_string()))
    } else {
        Version::default()
    };

    (name, version)
}

/// Extract environment variables from context
fn extract_environment(ctx: &ExtractionContext<'_>) -> HashMap<String, String> {
    let mut env = HashMap::new();

    // From CI/CD global environment
    for cicd_file in &ctx.cicd {
        env.extend(cicd_file.env.clone());
    }

    // From container environment
    for container in &ctx.containers {
        env.extend(container.environment.clone());
    }

    env
}

/// Infer tools from script commands
/// Example: "eslint ." -> Tool { name: "eslint", version: None }
fn infer_tools_from_scripts(scripts: &HashMap<String, String>) -> Vec<Tool> {
    let tool_keywords = [
        "eslint",
        "prettier",
        "rust-analyzer",
        "clippy",
        "rustfmt",
        "black",
        "pylint",
        "mypy",
        "pytest",
        "jest",
        "vitest",
        "cargo",
        "npm",
        "pnpm",
        "yarn",
    ];

    let mut tools = HashSet::new();

    for command in scripts.values() {
        for keyword in &tool_keywords {
            if command.contains(keyword) {
                tools.insert(keyword.to_string());
            }
        }
    }

    tools
        .into_iter()
        .map(|name| Tool {
            name,
            version: Version::default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::scan::Scan;
    use std::path::PathBuf;

    fn fixtures_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("analysis")
    }

    #[test]
    fn test_rust_workspace_analysis() {
        let rust_project = fixtures_path().join("rust");
        let scan = Scan::from(rust_project.clone());
        let result = Analysis::from(scan.into_iter());

        let pkg_config_dep = Dependencies(vec![
            Dependency { name: "pkg-config".to_string(), version: Version(None) },
        ]);

        let expected = Analysis(vec![
            RepoAnalysis {
                name: "rust".to_string(),
                path: rust_project.clone(),
                packages: Packages(vec![]),
                dev_tools: DevTools {
                    tools: vec![],
                    env: HashMap::new(),
                    shell_hook: None,
                    dependencies: pkg_config_dep.clone(),
                    services: Services(vec![]),
                },
                checks: Checks(vec![
                    Check {
                        name: "test".to_string(),
                        command: "make test".to_string(),
                        toolchain: Toolchain {
                            language: Language::Rust,
                            package_manager: PackageManager::Cargo,
                            version: Version(None),
                        },
                        dependencies: pkg_config_dep.clone(),
                        services: Services(vec![]),
                    },
                    Check {
                        name: "lint".to_string(),
                        command: "make lint".to_string(),
                        toolchain: Toolchain {
                            language: Language::Rust,
                            package_manager: PackageManager::Cargo,
                            version: Version(None),
                        },
                        dependencies: pkg_config_dep.clone(),
                        services: Services(vec![]),
                    },
                    Check {
                        name: "format".to_string(),
                        command: "make format".to_string(),
                        toolchain: Toolchain {
                            language: Language::Rust,
                            package_manager: PackageManager::Cargo,
                            version: Version(None),
                        },
                        dependencies: pkg_config_dep.clone(),
                        services: Services(vec![]),
                    },
                    Check {
                        name: "check".to_string(),
                        command: "make check".to_string(),
                        toolchain: Toolchain {
                            language: Language::Rust,
                            package_manager: PackageManager::Cargo,
                            version: Version(None),
                        },
                        dependencies: pkg_config_dep.clone(),
                        services: Services(vec![]),
                    },
                ]),
            },
            RepoAnalysis {
                name: "myapp-api".to_string(),
                path: rust_project.join("crates/myapp-api"),
                packages: Packages(vec![]),
                dev_tools: DevTools {
                    tools: vec![],
                    env: HashMap::new(),
                    shell_hook: None,
                    dependencies: Dependencies(vec![]),
                    services: Services(vec![]),
                },
                checks: Checks(vec![]),
            },
            RepoAnalysis {
                name: "myapp-cli".to_string(),
                path: rust_project.join("crates/myapp-cli"),
                packages: Packages(vec![]),
                dev_tools: DevTools {
                    tools: vec![],
                    env: HashMap::new(),
                    shell_hook: None,
                    dependencies: Dependencies(vec![]),
                    services: Services(vec![]),
                },
                checks: Checks(vec![]),
            },
            RepoAnalysis {
                name: "myapp-core".to_string(),
                path: rust_project.join("crates/myapp-core"),
                packages: Packages(vec![]),
                dev_tools: DevTools {
                    tools: vec![],
                    env: HashMap::new(),
                    shell_hook: None,
                    dependencies: Dependencies(vec![]),
                    services: Services(vec![]),
                },
                checks: Checks(vec![]),
            },
        ]);

        assert_eq!(result, expected);
    }

    #[test]
    fn test_js_python_monorepo_analysis() {
        let js_python_project = fixtures_path().join("js_and_python");
        let scan = Scan::from(js_python_project.clone());
        let result = Analysis::from(scan.into_iter());

        let shared_deps = Dependencies(vec![
            Dependency { name: "postgresql".to_string(), version: Version(None) },
        ]);

        let expected = Analysis(vec![RepoAnalysis {
            name: "js_and_python".to_string(),
            path: js_python_project.clone(),
            packages: Packages(vec![
                Package {
                    name: "fullstack-monorepo".to_string(),
                    path: js_python_project.clone(),
                    toolchain: Toolchain {
                        language: Language::JavaScript,
                        package_manager: PackageManager::Npm,
                        version: Version(Some(">=20.0.0".to_string())),
                    },
                    dependencies: shared_deps.clone(),
                    metadata: Metadata {
                        version: Version(Some("1.5.0".to_string())),
                        description: Some("Full-stack monorepo with Node.js API and Python ML service".to_string()),
                        authors: vec!["Development Team <dev@example.com>".to_string()],
                        license: Some("MIT".to_string()),
                    },
                },
                Package {
                    name: "ml-service".to_string(),
                    path: js_python_project.clone(),
                    toolchain: Toolchain {
                        language: Language::Python,
                        package_manager: PackageManager::Poetry,
                        version: Version(Some(">=3.11".to_string())),
                    },
                    dependencies: shared_deps.clone(),
                    metadata: Metadata {
                        version: Version(Some("0.3.2".to_string())),
                        description: Some("Machine learning service for predictions".to_string()),
                        authors: vec!["ML Team".to_string()],
                        license: Some("MIT".to_string()),
                    },
                },
            ]),
            dev_tools: DevTools {
                tools: vec![
                    Tool { name: "jest".to_string(), version: Version(None) },
                    Tool { name: "eslint".to_string(), version: Version(None) },
                    Tool { name: "prettier".to_string(), version: Version(None) },
                ],
                env: HashMap::new(),
                shell_hook: None,
                dependencies: shared_deps.clone(),
                services: Services(vec![]),
            },
            checks: Checks(vec![
                Check {
                    name: "test".to_string(),
                    command: "make test".to_string(),
                    toolchain: Toolchain {
                        language: Language::JavaScript,
                        package_manager: PackageManager::Npm,
                        version: Version(Some(">=20.0.0".to_string())),
                    },
                    dependencies: shared_deps.clone(),
                    services: Services(vec![]),
                },
                Check {
                    name: "lint".to_string(),
                    command: "make lint".to_string(),
                    toolchain: Toolchain {
                        language: Language::JavaScript,
                        package_manager: PackageManager::Npm,
                        version: Version(Some(">=20.0.0".to_string())),
                    },
                    dependencies: shared_deps.clone(),
                    services: Services(vec![]),
                },
                Check {
                    name: "format".to_string(),
                    command: "make format".to_string(),
                    toolchain: Toolchain {
                        language: Language::JavaScript,
                        package_manager: PackageManager::Npm,
                        version: Version(Some(">=20.0.0".to_string())),
                    },
                    dependencies: shared_deps.clone(),
                    services: Services(vec![]),
                },
                Check {
                    name: "check".to_string(),
                    command: "make check".to_string(),
                    toolchain: Toolchain {
                        language: Language::JavaScript,
                        package_manager: PackageManager::Npm,
                        version: Version(Some(">=20.0.0".to_string())),
                    },
                    dependencies: shared_deps.clone(),
                    services: Services(vec![]),
                },
                Check {
                    name: "test".to_string(),
                    command: "jest --coverage".to_string(),
                    toolchain: Toolchain {
                        language: Language::JavaScript,
                        package_manager: PackageManager::Npm,
                        version: Version(Some(">=20.0.0".to_string())),
                    },
                    dependencies: shared_deps.clone(),
                    services: Services(vec![]),
                },
                Check {
                    name: "format".to_string(),
                    command: "prettier --write \"**/*.{js,jsx,ts,tsx,json,md}\"".to_string(),
                    toolchain: Toolchain {
                        language: Language::JavaScript,
                        package_manager: PackageManager::Npm,
                        version: Version(Some(">=20.0.0".to_string())),
                    },
                    dependencies: shared_deps.clone(),
                    services: Services(vec![]),
                },
                Check {
                    name: "lint".to_string(),
                    command: "eslint . --ext .js,.jsx,.ts,.tsx".to_string(),
                    toolchain: Toolchain {
                        language: Language::JavaScript,
                        package_manager: PackageManager::Npm,
                        version: Version(Some(">=20.0.0".to_string())),
                    },
                    dependencies: shared_deps.clone(),
                    services: Services(vec![]),
                },
                Check {
                    name: "typecheck".to_string(),
                    command: "tsc --noEmit".to_string(),
                    toolchain: Toolchain {
                        language: Language::JavaScript,
                        package_manager: PackageManager::Npm,
                        version: Version(Some(">=20.0.0".to_string())),
                    },
                    dependencies: shared_deps.clone(),
                    services: Services(vec![]),
                },
            ]),
        }]);

        assert_eq!(result, expected);
    }
}
