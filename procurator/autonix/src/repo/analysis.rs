use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{mapping::{Language, PackageManager}, repo::scan::{Repo, ScanIter}};

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
/// This is the key intermediate representation that:
/// Contains everything needed to generate Nix expressions and can be compared with previous versions to detect changes
#[derive(Debug, Serialize, Deserialize)]
struct RepoConfiguration {
    /// Human-readable repo name (from manifest)
    name: String,

    /// Path from repository root to this repo
    path: PathBuf,

    /// Additional metadata from the manifest
    metadata: Metadata,

    /// Component type (library, binary, web service, etc.)
    component_type: ComponentType,

    /// Configuration for the toolchain, such as the language, version, etc.
    toolchain: ToolchainConfiguration,

    /// Configuration needed to build the repo
    build: BuildConfiguration,

    /// Configuration needed to check the repo. Checks are realted to nix flake's checks and can be run with `nix flake check`
    /// This is separate from build because some repos might not need to be built but can still have checks (e.g., linting, tests, etc.)
    check: CheckConfiguration,

    /// Configuration needed for the development environment
    develop: DevelopConfiguration,

    /// Dependencies needed for the repo to build and run
    dependencies: DependenciesConfiguration,

    /// Secrets, configurations, and other sensitive information needed for the repo to run
    runtime: RuntimeConfiguration,
}

impl From<Repo> for RepoConfiguration {
    fn from(repo: Repo) -> Self {
        todo!()
    }
}

#[derive(Debug, Serialize, Deserialize)]
enum ComponentType {
    /// Produces executable binary (goes in packages + apps)
    Binary,

    /// Library for other projects (no direct package output)
    Library,

    /// Full application, might have multiple entry points
    Application,
}

#[derive(Debug, Serialize, Deserialize)]
struct ToolchainConfiguration {
    /// The main language (Rust, Python, TypeScript, etc.)
    language: Language,

    /// Version constraint
    version: Version,

    /// Primary package manager
    package_manager: PackageManager,
}

#[derive(Debug, Serialize, Deserialize)]
struct BuildConfiguration {
        /// Native build inputs (nativeBuildInputs in Nix)
    /// Tools needed at build time that run on build machine
    native_inputs: Vec<NativeInput>,

    /// Build inputs (buildInputs in Nix)
    /// Libraries/deps needed at build time that are for target machine
    build_inputs: Vec<String>,

    /// Commands before main build
    pre_build: Vec<PhaseCommand>,

    /// Override main build command (usually use default)
    build_override: Option<String>,

    /// Commands after main build
    post_build: Vec<PhaseCommand>,

    /// Override install phase (usually automatic)
    install_override: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
struct CheckConfiguration {
    /// Check operations - each operation is self-contained
    /// Includes tests, lints, format checks, type checks, security audits
    checks: Vec<CheckOperation>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CheckOperation {
    /// What kind of check this is
    kind: CheckKind,

    /// The command to run
    /// Example: "cargo test", "eslint .", "mypy ."
    command: Command,

    /// Toolchain needed for THIS specific check
    /// None means use the primary toolchain from RepoConfiguration
    toolchain: Option<ToolchainConfiguration>,

    /// Dependencies needed for THIS check
    /// These are in addition to common dependencies in DependenciesConfiguration
    dependencies: Vec<Dependency>,

    /// System dependencies for THIS check
    system_dependencies: Vec<String>,

    /// Services needed for THIS check
    /// Example: postgresql for integration tests
    required_services: Vec<Service>,
}

#[derive(Debug, Serialize, Deserialize)]
enum CheckKind {
    Test,
    Lint,
    Format,
    TypeCheck,
    Security,
}



#[derive(Debug, Serialize, Deserialize)]
struct Command(String);

#[derive(Debug, Serialize, Deserialize)]
struct Version(Option<String>);



/// Metadata
///
/// Additional information from manifests that we might want to include
/// in the generated Nix file (meta.description, meta.license, etc.)
#[derive(Debug, Serialize, Deserialize)]
struct Metadata {
    /// repo version
    version: Version,

    /// repo description
    description: Option<String>,

    /// Authors/maintainers
    authors: Vec<String>,

    /// License
    license: Option<String>,
}



#[derive(Debug, Serialize, Deserialize)]
struct DevelopConfiguration {
    /// Packages needed in dev shell
    /// Includes the primary toolchain + any dev-specific tools
    /// Maps to: packages = [ ... ] or buildInputs/nativeBuildInputs
    packages: Vec<String>,

    /// Environment variables for development
    /// Maps to: env = { ... }
    environment: Vec<EnvVar>,

    /// Shell hook - runs when entering `nix develop`
    /// Maps to: shellHook = ''...'';
    shell_hook: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
struct EnvVar {
    name: String,
    default: Option<String>,
    description: Option<String>,
    required: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Service {
    name: String,
    version: Option<String>,
    /// Configuration needed to start service (especially for tests/dev)
    config: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DependenciesConfiguration {
    /// Runtime dependencies (needed when the app runs)
    /// Example: libraries linked at runtime, Python packages imported
    runtime: Vec<Dependency>,

    /// Build-time only dependencies
    /// Example: build tools, compilers that aren't in toolchain
    build_time: Vec<Dependency>,

    /// Development-only dependencies
    /// Example: test frameworks, linters (from package.json devDependencies)
    dev: Vec<Dependency>,

    /// System-level dependencies (OS packages)
    /// Maps to pkgs.* in Nix
    system: Vec<SystemDependency>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Dependency {
    name: String,
    version: Option<String>,
    source: DependencySource,
    /// Which package manager handles this
    manager: PackageManager,
}

#[derive(Debug, Serialize, Deserialize)]
struct RuntimeConfiguration {
    /// Entry points - how to run this
    entry_points: Vec<EntryPoint>,

    /// Environment variables needed at runtime
    environment: Vec<EnvVar>,

    /// CLI arguments/flags
    default_args: Vec<String>,

    /// Configuration files needed
    config_files: Vec<ConfigFile>,

    /// Services that must be running
    required_services: Vec<Service>,

    /// Exposed ports (for services/servers)
    ports: Vec<Port>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EntryPoint {
    /// Name of this entry point
    name: String,

    /// Path to executable/script
    path: PathBuf,

    /// Type of entry point
    kind: EntryKind,

    /// Arguments to pass
    default_args: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
enum EntryKind {
    Binary,           // Compiled executable
    Script,           // Interpreted script
    WebServer,        // HTTP server
    Worker,           // Background worker
    CLI,              // Command-line tool
    Service,          // Long-running service
}

#[derive(Debug, Serialize, Deserialize)]
struct ConfigFile {
    path: PathBuf,
    format: ConfigFormat,
    required: bool,
}

#[derive(Debug, Serialize, Deserialize)]
enum ConfigFormat {
    Toml,
    Yaml,
    Json,
    Env,  // .env file
    Custom(String),
}
