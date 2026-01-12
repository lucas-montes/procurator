// Parse the repository and discover its structure

use std::{
    collections::{HashMap, VecDeque},
    ffi::OsStr,
    ops::Not,
    path::{Path, PathBuf},
};

use crate::autonix::mapping::{Language, LockFile, ManifestFile};

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

fn should_ignore_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| IGNORED_DIR_BASENAMES.contains(&name))
        .unwrap_or(false)
}


#[derive(Debug)]
pub struct Scan {
    root: ScanNode,
}

impl From<PathBuf> for Scan {
    fn from(root: PathBuf) -> Self {
        Self { root: root.into() }
    }
}

#[derive(Debug)]
pub struct ScanNode {
    /// Path to this directory
    path: PathBuf,

    /// Files found in this directory
    files: DirectoryScan,

    /// Subdirectories
    children: Vec<ScanNode>,
}

impl From<PathBuf> for ScanNode {
    fn from(path: PathBuf) -> Self {
        let mut files = DirectoryScan::default();
        let mut children = Vec::new();

        // Read this directory
        if let Ok(entries) = std::fs::read_dir(&path) {
            for entry in entries.flatten() {
                let entry_path = entry.path();

                if entry_path.is_dir() {
                    // Skip ignored directories
                    if should_ignore_dir(&entry_path) {
                        continue;
                    }

                    // Recurse into subdirectory
                    children.push(entry_path.into());
                } else {
                    // Classify and add file to this node
                    files.add_file(entry_path);
                }
            }
        }

        ScanNode {
            path,
            files,
            children,
        }
    }
}

#[derive(Debug, Default)]
pub struct DirectoryScan {
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

impl DirectoryScan {
    fn add_file(&mut self, path: PathBuf) {
        match FileType::from(path) {
            FileType::Manifest(m) => self.manifest_files.push(m),
            FileType::Lockfile(l) => self.lockfiles.push(l),
            FileType::Buildfile(p) => self.buildfiles.push(BuildFile(p)),
            FileType::CicdFile(p) => self.cicd_files.push(CiCdFile(p)),
            FileType::Regular(lang) => {
                *self.file_per_language.entry(lang).or_insert(0) += 1;
            }
            FileType::Unknown => {}
        }
    }
}

#[derive(Debug)]
struct FilePath<T> {
    kind: T,
    path: PathBuf,
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
