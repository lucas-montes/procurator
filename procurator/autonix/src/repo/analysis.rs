use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    path::{Path, PathBuf},
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
#[derive(Debug)]
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
#[derive(Debug)]
struct RepoAnalysis {
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

#[derive(Debug)]
struct Packages(Vec<Package>);

impl From<&ExtractionContext<'_>> for Packages {
    fn from(ctx: &ExtractionContext<'_>) -> Self {
        todo!()
    }
}

/// A package that can be built from this repo
/// Examples: a binary crate in Cargo workspace, an app in npm workspace
#[derive(Debug)]
struct Package {
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
#[derive(Debug, Clone)]
struct Toolchain {
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
#[derive(Debug)]
struct Dependencies(Vec<Dependency>);

/// A dependency can be either a running service or a build-time package
#[derive(Debug)]
struct Dependency {
    name: String,
    version: Version,
}

#[derive(Debug)]
struct Tool {
    name: String,
    version: Version,
}

/// Development tools configuration
/// Maps to devShells.<system>.default in flake.nix
#[derive(Debug)]
struct DevTools {
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
        todo!()
    }
}

#[derive(Debug)]
struct Checks(Vec<Check>);

impl From<&ExtractionContext<'_>> for Checks {
    fn from(ctx: &ExtractionContext<'_>) -> Self {
        todo!()
    }
}

/// A check operation (test, lint, format check, etc.)
/// Maps to checks.<system>.<name> in flake.nix
#[derive(Debug)]
struct Check {
    /// Check name (e.g., "test", "lint", "format")
    name: String,

    /// Command to run (e.g., "cargo test", "eslint .")
    command: String,

    /// toolchain for this specific check
    toolchain: Toolchain,

    dependencies: Dependencies,
    services: Services,
}

#[derive(Debug)]
struct Services(Vec<Service>);

/// A service dependency (database, cache, message queue, etc.)
#[derive(Debug)]
struct Service {
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
#[derive(Debug, Clone)]
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
            .filter_map(|f| f.parse().ok())
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
