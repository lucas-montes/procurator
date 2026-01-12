use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::autonix::mapping::{Language, PackageManager};



/// Second pass: Project structure detection
///
/// Analyzes the files found in FirstPass to identify logical projects within the repository.
/// A repository can contain:
/// - A single project (most common)
/// - A monorepo with multiple workspace members (e.g., Cargo workspace, npm workspaces)
/// - Multiple independent projects (e.g., Tauri: Rust backend + JS frontend)
///
/// This pass groups manifests by directory, identifies workspace roots, and classifies
/// the repository structure without parsing file contents yet.
#[derive(Debug)]
pub struct Analysis {
    /// All detected projects in the repository
    /// Each project represents a buildable unit with its own language and dependencies
    projects: Vec<DetectedProject>,

    /// How the repository is structured
    /// Determines how we generate Nix code (single flake vs multiple packages)
    repository_type: RepositoryType,
}

/// Classification of repository structure
///
/// This determines our build and packaging strategy:
/// - SingleProject: Simple case, one language, one package
/// - Monorepo: Workspace with shared dependencies, build together
/// - MultiProject: Independent projects (different languages/purposes), build separately but in one flake
#[derive(Debug, Clone)]
enum RepositoryType {
    /// Single project in the repository
    SingleProject,

    /// Workspace/monorepo with multiple related packages
    Monorepo { workspace_root: PathBuf },

    /// Multiple independent projects (e.g., fullstack app with separate frontend/backend)
    /// Different languages, separate build processes, but combined in one repository
    MultiProject,
}

/// A detected project within the repository
///
/// Represents a single buildable unit with its own language, dependencies, and build process.
/// In a monorepo, each workspace member becomes a DetectedProject.
/// In a multi-language repo (like Tauri), each language component is a separate DetectedProject.
#[derive(Debug, Clone)]
struct DetectedProject {
    /// Path relative to repository root where this project lives
    /// For root projects this is ".", for nested it might be "packages/frontend"
    root: PathBuf,

    /// Primary programming language of this project
    /// Determines which Nix builder to use (buildRustPackage, buildNpmPackage, etc.)
    language: Language,

    /// Package manager to use for dependency installation
    /// Inferred from lockfiles (yarn.lock → yarn) or manifest hints
    package_manager: PackageManager,

    /// The main manifest file that declares this project
    /// This is where we'll extract name, version, dependencies
    primary_manifest: PathBuf,

    /// Additional manifests for multi-language projects
    /// Example: A Rust project might also have a package.json for WASM bindings
    secondary_manifests: Vec<PathBuf>,

    /// What kind of project this is (affects how we package it)
    project_type: ProjectType,
}

/// Type of project artifact
///
/// Determines what we build and how we install it:
/// - Application: Install binaries to $out/bin
/// - Library: Install libraries for other packages to use
/// - Both: Some Rust crates have both lib and bins
#[derive(Debug, Clone)]
enum ProjectType {
    /// Produces executable binaries
    Application,

    /// Produces a library for other code to depend on
    Library,

    /// Produces both (common in Rust with lib + bin targets)
    Both,
}

/// Third pass: Dependency resolution and configuration extraction
///
/// Parses manifest files to extract detailed project configuration.
/// This is the INTERMEDIATE REPRESENTATION that we save and reuse.
///
/// Why this is separate from SecondPass:
/// - SecondPass is fast (just file analysis)
/// - IntermediateRepresentation is slower (parses TOML/JSON, reads file contents)
/// - We can cache IntermediateRepresentation results and only re-run when manifests change
/// - Makes it easy to compare configurations for drift detection
#[derive(Debug, Serialize, Deserialize)]
struct IntermediateRepresentation {
    /// Complete configuration for each project in the repository
    /// This is what we'll use to generate flake.nix
    projects: Vec<ProjectConfiguration>,
}

/// Complete configuration for a single project
///
/// This is the key intermediate representation that:
/// 1. Can be serialized to .procurator/config.json for caching
/// 2. Contains everything needed to generate Nix expressions
/// 3. Is language-agnostic (works for any language we support)
/// 4. Can be compared with previous versions to detect changes
#[derive(Debug, Serialize, Deserialize)]
struct ProjectConfiguration {
    /// Human-readable project name (from manifest)
    name: String,

    /// Path from repository root to this project
    path: PathBuf,

    /// Language toolchain requirements (compiler, version, package manager)
    toolchain: Toolchain,

    /// Build system tools needed beyond the language toolchain
    build_system: BuildSystem,

    /// Language-level dependencies (managed by package manager)
    /// We don't store all deps, just metadata and hints about system requirements
    language_dependencies: LanguageDependencies,

    /// System-level dependencies that Nix must provide
    /// These are the packages we'll add to buildInputs in Nix
    system_dependencies: SystemDependencies,

    /// How to build this project
    build_config: BuildConfiguration,

    /// How to test this project (optional)
    test_config: Option<TestConfiguration>,

    /// Additional metadata from the manifest
    metadata: ProjectMetadata,
}

/// Language toolchain specification
///
/// Tells Nix which language runtime and package manager to provide.
/// Example: { language: Rust, version: "1.75", package_manager: Cargo }
///       → Use rustc 1.75 and cargo in the build environment
#[derive(Debug, Serialize, Deserialize)]
struct Toolchain {
    /// Programming language
    language: Language,

    /// Version constraint if specified in manifest
    /// Example: "rust-version = 1.70" in Cargo.toml
    ///       or "engines.node = >=18" in package.json
    version: Option<String>,

    /// Package manager (cargo, npm, yarn, poetry, etc.)
    package_manager: PackageManager,

    /// Package manager version if locked
    /// Some manifests specify this (packageManager in package.json)
    package_manager_version: Option<String>,
}

/// Build system tools required
///
/// Beyond the language toolchain, what other build tools are needed?
/// Example: A Rust project with a build.rs might need cmake or protobuf
#[derive(Debug, Serialize, Deserialize)]
struct BuildSystem {
    /// Primary build tool (cargo, npm, go, mvn, etc.)
    /// This is usually the package manager's build command
    primary: String,

    /// Additional build tools discovered from build files
    /// Examples: make, cmake, autoconf, protoc, wasm-pack
    additional: Vec<String>,
}

/// Language-level dependencies summary
///
/// We don't store all package.json dependencies here (that's the package manager's job).
/// Instead, we store metadata about dependencies and hints about system requirements.
///
/// Why? Because we only care about dependencies that require system packages from Nix.
/// Example: openssl-sys in Rust → need openssl from nixpkgs
#[derive(Debug, Serialize, Deserialize)]
struct LanguageDependencies {
    /// Whether the project has runtime dependencies
    has_dependencies: bool,

    /// Whether it has dev dependencies (for testing, linting, etc.)
    has_dev_dependencies: bool,

    /// Whether it has build-time dependencies
    has_build_dependencies: bool,

    /// Dependencies that hint at native/system requirements
    /// Examples:
    /// - Rust: ["openssl-sys", "libsqlite3-sys"] → need openssl, sqlite
    /// - Node: ["node-gyp", "canvas"] → need python3, cairo
    /// - Python: ["psycopg2"] → need postgresql
    native_binding_hints: Vec<String>,
}

/// System dependencies that Nix must provide
///
/// These become buildInputs, nativeBuildInputs, etc. in the Nix expression.
/// We infer these from:
/// - Native dependency hints (openssl-sys → openssl)
/// - Build files (CMakeLists.txt mentions libfoo)
/// - Common patterns (node-gyp always needs python3)
#[derive(Debug, Serialize, Deserialize)]
struct SystemDependencies {
    /// Libraries needed at build time
    /// Example: openssl, sqlite (for linking)
    build_inputs: Vec<NixPackage>,

    /// Libraries needed at runtime
    /// Example: dynamic libraries that must be available when running
    runtime_inputs: Vec<NixPackage>,

    /// Native build tools (compilers, code generators)
    /// Example: pkg-config, cmake, protobuf compiler
    native_build_inputs: Vec<NixPackage>,
}

/// A Nix package dependency with confidence and reasoning
///
/// We track why we think each package is needed and how confident we are.
/// This allows users to review and override our inferences.
#[derive(Debug, Serialize, Deserialize)]
struct NixPackage {
    /// Package name in nixpkgs (e.g., "openssl", "pkg-config")
    name: String,

    /// Why we think this package is needed
    /// Helps users understand and debug our inference
    reason: DependencyReason,

    /// Confidence score (0.0 to 1.0)
    /// 1.0 = certain (explicit mention)
    /// 0.5 = probable (common pattern)
    /// 0.3 = possible (heuristic guess)
    confidence: f32,
}

/// Why we inferred a system dependency
///
/// Provides transparency in our detection logic.
/// Users can see "I need openssl because of openssl-sys crate"
#[derive(Debug, Serialize, Deserialize)]
enum DependencyReason {
    /// Found a -sys crate in Cargo.toml
    SysCrate(String),

    /// Found a native Node module
    NativeModule(String),

    /// Mentioned in a build file (CMakeLists.txt, etc.)
    BuildFile(String),

    /// Known pattern (e.g., node-gyp always needs python)
    CommonPattern(String),
}

/// Build configuration
///
/// How to actually build this project once we have all dependencies.
/// We try to extract this from:
/// 1. CI/CD files (what commands actually work in practice)
/// 2. Package.json scripts
/// 3. Standard conventions (cargo build, npm run build, etc.)
#[derive(Debug, Serialize, Deserialize)]
struct BuildConfiguration {
    /// Build command to run
    /// Example: "cargo build --release"
    command: Option<String>,

    /// What artifacts this build produces
    /// We need to know what to copy to $out
    outputs: Vec<BuildOutput>,

    /// Environment variables needed during build
    /// Example: { "CARGO_BUILD_JOBS": "4" }
    env_vars: std::collections::HashMap<String, String>,
}

/// A build output artifact
///
/// Tells Nix what to install and where to put it.
/// Example: Binary "my-app" → install to $out/bin/my-app
#[derive(Debug, Serialize, Deserialize)]
struct BuildOutput {
    /// What kind of output
    kind: OutputKind,

    /// Name of the artifact
    name: String,

    /// Path relative to build directory
    path: PathBuf,
}

/// Type of build output
#[derive(Debug, Serialize, Deserialize)]
enum OutputKind {
    /// Executable binary
    Binary,

    /// Shared/static library
    Library,

    /// Archive file
    Archive,
}

/// Test configuration
///
/// How to run tests for this project.
/// We extract this from package.json scripts, CI files, etc.
#[derive(Debug, Serialize, Deserialize)]
struct TestConfiguration {
    /// Command to run tests
    /// Example: "cargo test", "npm test"
    command: String,

    /// Whether tests need a database
    /// If true, we need to spin up a test database in the Nix check phase
    needs_database: bool,

    /// Whether tests need network access
    /// Nix sandbox blocks network by default, we need to allow it
    needs_network: bool,
}

/// Project metadata
///
/// Additional information from manifests that we might want to include
/// in the generated Nix file (meta.description, meta.license, etc.)
#[derive(Debug, Serialize, Deserialize)]
struct ProjectMetadata {
    /// Project version
    version: Option<String>,

    /// Project description
    description: Option<String>,

    /// Authors/maintainers
    authors: Vec<String>,

    /// License
    license: Option<String>,
}
