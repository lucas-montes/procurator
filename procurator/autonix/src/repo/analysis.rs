use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    mapping::{Language, PackageManager},
    repo::scan::{Repo, ScanIter},
};

/// Analysis result for the entire repository
/// Complete configuration for each repo in the repository
/// This is what we'll use to generate flake.nix
#[derive(Debug, Serialize, Deserialize)]
pub struct Analysis(Vec<RepoConfiguration>);

impl From<ScanIter> for Analysis {
    fn from(scan: ScanIter) -> Self {
        Self(Vec::from_iter(scan.map(RepoConfiguration::from)))
    }
}

/// Complete configuration for a single repo
///
/// A repo can contain multiple packages (e.g., Rust workspace, npm workspaces)
/// This is the key intermediate representation that contains everything needed
/// to generate Nix expressions and can be compared with previous versions to detect changes
#[derive(Debug, Serialize, Deserialize)]
struct RepoConfiguration {
    /// Human-readable repo name (from manifest)
    name: String,

    /// Path from repository root to this repo
    path: PathBuf,

    /// Packages produced by this repo
    /// One repo can have multiple packages (workspace members, monorepo apps)
    packages: Packages,

    /// Dependencies needed for the repo to build and run
    dependencies: Dependencies,

    /// Development environment configuration
    dev_tools: DevTools,

    /// Check operations (tests, lints, formatting, etc.)
    checks: Checks,

    /// Runtime configuration (how to run the built app)
    /// None for libraries that don't produce runnable artifacts
    runtime: Option<Runtime>,

    /// Additional metadata from the manifest
    metadata: Metadata,
}

impl From<Repo> for RepoConfiguration {
    fn from(repo: Repo) -> Self {
        // Parse all manifest files
        let parsed_manifests: Vec<_> = repo
            .manifest_files()
            .iter()
            .filter_map(|manifest_file| manifest_file.parse().ok())
            .collect();

        // Get primary manifest or use defaults
        let primary_manifest = parsed_manifests.first();

        // Extract name from primary manifest or directory
        let name = primary_manifest
            .and_then(|m| m.names.first())
            .cloned()
            .unwrap_or_else(|| {
                repo.path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            });

        // Extract metadata from primary manifest
        let metadata = if let Some(manifest) = primary_manifest {
            Metadata {
                version: Version(manifest.version.clone()),
                description: manifest.metadata.description.clone(),
                authors: manifest.metadata.authors.clone(),
                license: manifest.metadata.license.clone(),
            }
        } else {
            Metadata {
                version: Version(None),
                description: None,
                authors: Vec::new(),
                license: None,
            }
        };

        // TODO: Implement full parsing logic
        // - Extract packages from workspace members
        // - Parse lockfiles for dependencies
        // - Infer toolchain from manifests
        // - Discover checks from CI/CD files
        // - Detect runtime configuration

        RepoConfiguration {
            name,
            path: repo.path().to_path_buf(),
            packages: Packages(Vec::new()),
            dependencies: Dependencies {
                nix_packages: Vec::new(),
                language_deps: HashMap::new(),
            },
            dev_tools: DevTools {
                tools: Vec::new(),
                env: HashMap::new(),
                shell_hook: None,
            },
            checks: Checks(Vec::new()),
            runtime: None,
            metadata,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Packages(Vec<Package>);

/// A package that can be built from this repo
/// Examples: a binary crate in Cargo workspace, an app in npm workspace
#[derive(Debug, Serialize, Deserialize)]
struct Package {
    /// Package name
    name: String,

    /// Path relative to repo root
    path: PathBuf,
    toolchain: Toolchain,
    /// Custom build command if needed
    /// Example: "cargo build -p api", "npm run build --workspace apps/web"
    /// None means use default build for the package manager
    build_command: Option<String>,
}

/// Toolchain configuration for the repo
#[derive(Debug, Serialize, Deserialize)]
struct Toolchain {
    /// The main language (Rust, Python, TypeScript, etc.)
    language: Language,

    /// Primary package manager
    package_manager: PackageManager,

    /// Version constraint from manifest (e.g., "1.75.0", ">=3.11", "20.x")
    version: Version,
}

/// Dependencies needed for the repo
#[derive(Debug, Serialize, Deserialize)]
struct Dependencies {
    /// System packages from nixpkgs
    /// Examples: pkgs.openssl, pkgs.postgresql, pkgs.pkg-config
    /// Maps to buildInputs/nativeBuildInputs in Nix
    nix_packages: Vec<String>,

    /// Language-level dependencies from lockfiles
    /// Used for change detection and caching
    /// Key: package name, Value: version
    language_deps: HashMap<String, String>,
}

/// Development tools configuration
/// Maps to devShells.<system>.default in flake.nix
#[derive(Debug, Serialize, Deserialize)]
struct DevTools {
    /// Development tools (rust-analyzer, prettier, eslint, etc.)
    /// These are in addition to the base toolchain
    tools: Vec<String>,

    /// Environment variables for development
    env: HashMap<String, String>,

    /// Shell hook - runs when entering `nix develop`
    shell_hook: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Checks(Vec<Check>);

/// A check operation (test, lint, format check, etc.)
/// Maps to checks.<system>.<name> in flake.nix
#[derive(Debug, Serialize, Deserialize)]
struct Check {
    /// Check name (e.g., "test", "lint", "format")
    name: String,

    /// Command to run (e.g., "cargo test", "eslint .")
    command: String,

    /// toolchain for this specific check
    toolchain: Toolchain,

    /// Services needed for this check (e.g., postgresql for integration tests)
    services: Vec<Service>,
}

/// Runtime configuration for running the built application
/// Maps to apps.<system>.* in flake.nix
#[derive(Debug, Serialize, Deserialize)]
struct Runtime {
    /// How to run the application (binaries, scripts, servers)
    entry_points: Vec<EntryPoint>,

    /// Environment variables needed at runtime
    env: HashMap<String, String>,

    /// Services that must be running (databases, caches, etc.)
    services: Vec<Service>,

    /// Ports this service exposes
    ports: Vec<u16>,
}

/// An entry point for running the application
#[derive(Debug, Serialize, Deserialize)]
struct EntryPoint {
    /// Entry point name
    name: String,

    /// Path to executable/script relative to package root
    path: PathBuf,

    /// Default arguments to pass
    default_args: Vec<String>,
}

/// A service dependency (database, cache, message queue, etc.)
#[derive(Debug, Serialize, Deserialize)]
struct Service {
    /// Service name (postgresql, redis, mongodb, etc.)
    name: String,

    /// Version constraint if any
    version: Version,

    /// Configuration/setup needed to start the service
    /// TODO: maybe a Path would be better
    config: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Version(Option<String>);

/// Metadata from manifest files
/// Maps to meta.* in Nix derivation
#[derive(Debug, Serialize, Deserialize)]
struct Metadata {
    /// Package version
    version: Version,

    /// Description
    description: Option<String>,

    /// Authors/maintainers
    authors: Vec<String>,

    /// License
    license: Option<String>,
}
