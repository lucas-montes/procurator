// The parser module looks for the configuration of a repository. Think of it as a mix between
// railpack and direnv.
use std::{
    collections::{HashMap, VecDeque},
    ffi::OsStr,
    ops::Not,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::autonix::mapping::{Language, LockFile, ManifestFile, PackageManager};

const IGNORED_DIR_BASENAMES: [&str; 32] = [
    // VCS
    ".git",
    ".hg",
    ".svn",
    // JavaScript
    "node_modules",
    ".yarn",
    ".pnpm-store",
    ".turbo",
    ".nx",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".parcel-cache",
    // Rust
    "target",
    // Python
    ".venv",
    "venv",
    "env",
    "__pycache__",
    ".tox",
    ".nox",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    // General build/artifacts
    "dist",
    "build",
    "out",
    "coverage",
    ".cache",
    ".direnv",
    ".idea",
    ".vscode",
    "vendor",
    ".terraform",
];

#[derive(Debug)]
struct FilePath<T> {
    kind: T,
    path: PathBuf,
}

#[derive(Debug)]
pub struct Files {
    /// All manifest files found in the repository that can tell us what dependencies are needed
    manifest_files: Vec<FilePath<ManifestFile>>,
    /// All lockfiles found in the repository that cann tell use what dependencies to use and what packages
    lockfiles: Vec<FilePath<LockFile>>,
    /// All buildfiles found in the repository that can tell us how to build the project
    buildfiles: Vec<BuildFile>,
    /// CI/CD files found in the repository that can tell us how the project is deployed/tested or even built
    cicd_files: Vec<CiCdFile>,
    /// The number of files with the extension of each language found in the repository
    file_per_language: HashMap<Language, u16>, // NOTE: maybe if we found one file of a lanauge, like python, it's just a script to build or execute some tests, we might want to know that we need the interpreter installed?
}

impl FromIterator<PathBuf> for Files {
    fn from_iter<T: IntoIterator<Item = PathBuf>>(iter: T) -> Self {
        let mut manifest_files = Vec::new();
        let mut lockfiles = Vec::new();
        let mut buildfiles = Vec::new();
        let mut cicd_files = Vec::new();
        let mut file_per_language = HashMap::new();

        for path in iter.into_iter().map(FileType::from) {
            match path {
                FileType::Manifest(manifest) => manifest_files.push(manifest),
                FileType::Lockfile(lockfile) => lockfiles.push(lockfile),
                FileType::Buildfile(path) => buildfiles.push(BuildFile(path)),
                FileType::CicdFile(path) => cicd_files.push(CiCdFile(path)),
                FileType::Regular(language) => {
                    *file_per_language.entry(language).or_insert(0) += 1;
                }
                FileType::Unknown => {}
            }
        }

        Files {
            manifest_files,
            lockfiles,
            buildfiles,
            cicd_files,
            file_per_language,
        }
    }
}


#[derive(Debug)]
pub struct BuildFile(PathBuf);

impl BuildFile {
    fn is_buildfile(filename: &str) -> bool {
        matches!(
            filename,
            "Makefile"
                | "GNUmakefile"
                | "makefile"
                | "CMakeLists.txt"
                | "meson.build"
                | "BUILD"
                | "BUILD.bazel"
                | "WORKSPACE"
                | "Rakefile"
                | "build.xml"
                | "build.zig"
                | "justfile"
                | "Taskfile.yml"
        )
    }
}

#[derive(Debug)]
pub struct CiCdFile(PathBuf);

impl CiCdFile {
    fn is_cicd_file(path: &str) -> bool {
        // GitHub Actions - .github/workflows/*.yml or *.yaml
        if path.contains(".github/workflows/")
            && (path.ends_with(".yml") || path.ends_with(".yaml"))
        {
            return true;
        }

        // GitLab CI
        if path.ends_with(".gitlab-ci.yml") || path.ends_with(".gitlab-ci.yaml") {
            return true;
        }

        // CircleCI - .circleci/config.yml or any yml in .circleci/
        if path.contains(".circleci/") && (path.ends_with(".yml") || path.ends_with(".yaml")) {
            return true;
        }

        // Travis CI
        if path.ends_with(".travis.yml") {
            return true;
        }

        // Jenkins
        if path.ends_with("Jenkinsfile") || path.contains("Jenkinsfile") {
            return true;
        }

        // Azure Pipelines
        if path.ends_with("azure-pipelines.yml") || path.ends_with("azure-pipelines.yaml") {
            return true;
        }

        // Bitbucket Pipelines
        if path.ends_with("bitbucket-pipelines.yml") {
            return true;
        }

        // Drone CI
        if path.ends_with(".drone.yml") || path.ends_with(".drone.yaml") {
            return true;
        }

        // Buildkite
        if path.contains(".buildkite/") && (path.ends_with(".yml") || path.ends_with(".yaml")) {
            return true;
        }
        if path.ends_with("buildkite.yml") || path.ends_with("buildkite.yaml") {
            return true;
        }

        // AppVeyor
        if path.ends_with("appveyor.yml") || path.ends_with(".appveyor.yml") {
            return true;
        }

        // Wercker
        if path.ends_with("wercker.yml") {
            return true;
        }

        // Woodpecker CI
        if path.contains(".woodpecker/") && (path.ends_with(".yml") || path.ends_with(".yaml")) {
            return true;
        }

        // Concourse CI
        if path.ends_with("pipeline.yml") && path.contains("ci/") {
            return true;
        }

        // TeamCity
        if path.contains(".teamcity/") {
            return true;
        }

        // Codefresh
        if path.ends_with("codefresh.yml") {
            return true;
        }

        // Semaphore CI
        if path.contains(".semaphore/") && (path.ends_with(".yml") || path.ends_with(".yaml")) {
            return true;
        }

        // Buddy
        if path.ends_with("buddy.yml") {
            return true;
        }

        // Shippable
        if path.ends_with("shippable.yml") {
            return true;
        }

        // CodeShip
        if path.contains("codeship-") && (path.ends_with(".yml") || path.ends_with(".yaml")) {
            return true;
        }

        false
    }
}


enum FileType {
    Manifest(FilePath<ManifestFile>),
    Lockfile(FilePath<LockFile>),
    Buildfile(PathBuf),
    CicdFile(PathBuf),
    Regular(Language),
    Unknown,
}

impl From<PathBuf> for FileType {
    fn from(path: PathBuf) -> Self {
        let Some(filename) = path.file_name().and_then(OsStr::to_str) else {
            return Self::Unknown;
        };

        if let Ok(manifest) = ManifestFile::try_from(filename) {
            return Self::Manifest(FilePath {
                kind: manifest,
                path,
            });
        }

        if let Ok(lockfile) = LockFile::try_from(filename) {
            return Self::Lockfile(FilePath {
                kind: lockfile,
                path,
            });
        }

        if path.to_str().is_some_and(CiCdFile::is_cicd_file) {
            return Self::CicdFile(path);
        }

        if BuildFile::is_buildfile(filename) {
            return Self::Buildfile(path);
        }

        if let Some(language) = path
            .extension()
            .and_then(OsStr::to_str)
            .and_then(Language::from_extension)
        {
            return Self::Regular(language);
        }

        Self::Unknown
    }
}

#[derive(Debug)]
pub struct Parser<T = PathBuf>(T);

impl Parser<PathBuf> {
    pub fn new(path: PathBuf) -> Self {
        tracing::info!("Parsing repository: {path:?}");
        Self(path)
    }

    pub fn pass(self) -> Parser<Files> {
        tracing::info!("Running first pass");
        Parser(DirectoryIterator::from(self.0).collect())
    }
}

impl Parser<Files> {
    pub fn pass(self) -> Parser<SecondPass> {
        tracing::info!("Running second pass: detecting project structure");

        // Group manifests by directory to find project roots
        let mut projects_by_dir: HashMap<PathBuf, Vec<&FilePath<ManifestFile>>> = HashMap::new();
        for manifest in &self.0.manifest_files {
            if let Some(parent) = manifest.path.parent() {
                projects_by_dir
                    .entry(parent.to_path_buf())
                    .or_default()
                    .push(manifest);
            }
        }

        // Detect projects from the grouped manifests
        let mut projects = Vec::new();
        for (dir, manifests) in projects_by_dir {
            if let Some(project) = Self::detect_project_in_dir(&dir, manifests, &self.0) {
                projects.push(project);
            }
        }

        // Classify repository type based on detected projects
        let repository_type = Self::classify_repository(&projects, &self.0);

        tracing::info!(
            "Detected {} projects with repository type: {:?}",
            projects.len(),
            repository_type
        );

        Parser(SecondPass {
            projects,
            repository_type,
        })
    }

    /// Detect a project in a specific directory based on its manifest files
    fn detect_project_in_dir(
        dir: &Path,
        manifests: Vec<&FilePath<ManifestFile>>,
        first_pass: &Files,
    ) -> Option<DetectedProject> {
        // Priority: Use the most authoritative manifest for each language
        let primary_manifest = manifests.first()?;
        let language = Language::from(&primary_manifest.kind);

        // Find corresponding lockfile to determine package manager
        let package_manager = Self::infer_package_manager(dir, &language, first_pass);

        // Determine project type (application, library, or both)
        let project_type = Self::determine_project_type(&primary_manifest.kind);

        Some(DetectedProject {
            root: dir.to_path_buf(),
            language,
            package_manager,
            primary_manifest: primary_manifest.path.clone(),
            secondary_manifests: manifests[1..].iter().map(|m| m.path.clone()).collect(),
            project_type,
        })
    }

    /// Infer the package manager from lockfiles or manifest hints
    fn infer_package_manager(
        dir: &Path,
        language: &Language,
        first_pass: &Files,
    ) -> PackageManager {
        // Look for lockfiles in the same directory
        for lockfile in &first_pass.lockfiles {
            if lockfile.path.parent() == Some(dir) {
                // Infer package manager from lockfile
                let lang_from_lock = Language::from(&lockfile.kind);
                if lang_from_lock == *language {
                    return PackageManager::try_from(&lockfile.kind)
                        .unwrap_or_else(|_| Self::default_package_manager(language));
                }
            }
        }

        // No lockfile found, use default for the language
        Self::default_package_manager(language)
    }

    /// Get the default package manager for a language
    fn default_package_manager(language: &Language) -> PackageManager {
        match language {
            Language::Rust => PackageManager::Cargo,
            Language::JavaScript => PackageManager::Npm,
            Language::Python => PackageManager::Pip,
            Language::Go => PackageManager::GoModules,
            Language::Java => PackageManager::Maven,
            Language::CSharp => PackageManager::Nuget,
            Language::Ruby => PackageManager::Bundler,
            Language::PHP => PackageManager::Composer,
            Language::C => PackageManager::Cargo, // Placeholder
            Language::Bash => PackageManager::Cargo, // Placeholder
        }
    }

    /// Determine if this is an application, library, or both
    fn determine_project_type(manifest: &ManifestFile) -> ProjectType {
        match manifest {
            // Rust can be both (has lib.rs and src/main.rs)
            ManifestFile::CargoToml => ProjectType::Both,

            // Most others default to application
            _ => ProjectType::Application,
        }
    }

    /// Classify the repository structure
    fn classify_repository(projects: &[DetectedProject], first_pass: &Files) -> RepositoryType {
        if projects.is_empty() {
            return RepositoryType::SingleProject;
        }

        if projects.len() == 1 {
            return RepositoryType::SingleProject;
        }

        // Check if this is a workspace/monorepo
        // Look for workspace indicators at the root
        for manifest in &first_pass.manifest_files {
            // Cargo workspace indicated by workspace.members in root Cargo.toml
            if matches!(manifest.kind, ManifestFile::CargoToml) {
                // TODO: Actually parse to check for [workspace] section
                // For now, assume multiple Cargo.toml = workspace
                let cargo_count = first_pass
                    .manifest_files
                    .iter()
                    .filter(|m| matches!(m.kind, ManifestFile::CargoToml))
                    .count();
                if cargo_count > 1 {
                    return RepositoryType::Monorepo {
                        workspace_root: manifest.path.clone(),
                    };
                }
            }

            // npm/yarn workspace in root package.json
            if matches!(manifest.kind, ManifestFile::PackageJson) {
                // TODO: Parse package.json to check for workspaces field
                let package_json_count = first_pass
                    .manifest_files
                    .iter()
                    .filter(|m| matches!(m.kind, ManifestFile::PackageJson))
                    .count();
                if package_json_count > 1 {
                    return RepositoryType::Monorepo {
                        workspace_root: manifest.path.clone(),
                    };
                }
            }
        }

        // Multiple projects with different languages = multi-project
        let languages: std::collections::HashSet<_> =
            projects.iter().map(|p| &p.language).collect();
        if languages.len() > 1 {
            return RepositoryType::MultiProject;
        }

        // Default to monorepo for multiple projects of same language
        RepositoryType::Monorepo {
            workspace_root: projects.first().unwrap().root.clone(),
        }
    }
}

impl Parser<SecondPass> {
    pub fn pass(self) -> Parser<IntermediateRepresentation> {
        tracing::info!("Running third pass: extracting project configuration");

        let projects: Vec<ProjectConfiguration> = self
            .0
            .projects
            .into_iter()
            .map(Self::build_project_configuration)
            .collect();

        tracing::info!("Built configuration for {} projects", projects.len());

        Parser(IntermediateRepresentation { projects })
    }

    /// Build a complete project configuration from a detected project
    fn build_project_configuration(detected: DetectedProject) -> ProjectConfiguration {
        // For now, create a basic configuration
        // TODO: Actually parse manifest files to extract this information

        let name = detected
            .root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        ProjectConfiguration {
            name,
            path: detected.root,
            toolchain: Toolchain {
                language: detected.language.clone(),
                version: None, // TODO: Extract from manifest
                package_manager: detected.package_manager.clone(),
                package_manager_version: None, // TODO: Extract if specified
            },
            build_system: BuildSystem {
                primary: format!("{:?}", detected.package_manager).to_lowercase(),
                additional: vec![], // TODO: Detect from build files
            },
            language_dependencies: LanguageDependencies {
                has_dependencies: true,        // TODO: Parse manifest
                has_dev_dependencies: false,   // TODO: Parse manifest
                has_build_dependencies: false, // TODO: Parse manifest
                native_binding_hints: vec![],  // TODO: Parse manifest for -sys crates, etc.
            },
            system_dependencies: SystemDependencies {
                build_inputs: vec![],
                runtime_inputs: vec![],
                native_build_inputs: vec![],
            },
            build_config: BuildConfiguration {
                command: None, // TODO: Extract from CI files or scripts
                outputs: vec![],
                env_vars: HashMap::new(),
            },
            test_config: None, // TODO: Detect test configuration
            metadata: ProjectMetadata {
                version: None,
                description: None,
                authors: vec![],
                license: None,
            },
        }
    }
}

impl Parser<IntermediateRepresentation> {
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(&self.0)?;
        std::fs::write(path, json)?;
        tracing::info!("Saved configuration to {path:?}");
        Ok(())
    }

    pub fn print(&self){
        tracing::info!(?self, "Intermediate Representation:");
        tracing::info!("Detected {} projects", self.0.projects.len());
        for config in &self.0.projects {
            tracing::info!(
                "  - {} ({:?}, {:?})",
                config.name,
                config.toolchain.language,
                config.toolchain.package_manager
            );
        }
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let json = std::fs::read(path)?;
        let ir = serde_json::from_slice(&json)?;
        tracing::info!("Loaded configuration from {path:?}");
        Ok(Parser(ir))
    }
}

struct DirectoryIterator {
    queue: VecDeque<PathBuf>,
}

impl From<PathBuf> for DirectoryIterator {
    fn from(root: PathBuf) -> Self {
        Self {
            queue: VecDeque::from([root.clone()]),
        }
    }
}

impl DirectoryIterator {
    fn should_ignore_dir(path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| IGNORED_DIR_BASENAMES.contains(&name))
            .unwrap_or(false)
    }
}

impl Iterator for DirectoryIterator {
    type Item = PathBuf;
    fn next(&mut self) -> Option<Self::Item> {
        self.queue.pop_front().inspect(|p| {
            if p.is_dir() && Self::should_ignore_dir(p).not() {
                if let Ok(entries) = std::fs::read_dir(p) {
                    for entry in entries.flatten() {
                        self.queue.push_back(entry.path());
                    }
                }
            }
        })
    }
}
