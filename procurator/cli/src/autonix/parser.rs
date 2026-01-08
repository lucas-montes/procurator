use std::{
    collections::{HashMap, VecDeque},
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


struct FilePath<T>{
    kind: T,
    path: PathBuf,
}

struct FirstPass {
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

struct BuildFile(PathBuf);

impl TryFrom<PathBuf> for BuildFile {
    type Error = ();
    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        path.ends_with(child)
        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
            return Err(());
        };
        if Language::from_buildfile_filename(filename).is_some() {
            Ok(BuildFile)
        } else {
            Err(())
        }
    }
}

struct CiCdFile(PathBuf);


enum FileType {
    Manifest(FilePath<ManifestFile>),
    Lockfile(FilePath<LockFile>),
    Buildfile(PathBuf),
    CicdFile(PathBuf),
    Regular(Language),
    Unknown(PathBuf)
}

impl From<PathBuf> for FileType {
    fn from(path: PathBuf) -> Self {
        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
            return Self::Unknown(path)
        }
            if let Some(manifest_kind) = ManifestFile::from_filename(filename) {
                return FileType::Manifest(FilePath {
                    kind: manifest_kind,
                    path,
                });
            }
            if let Some(lockfile_kind) = LockFile::from_filename(filename) {
                return FileType::Lockfile(FilePath {
                    kind: lockfile_kind,
                    path,
                });
            }
            if let Some(buildfile_kind) = Language::from_buildfile_filename(filename) {
                return FileType::Buildfile(path);
            }
            if CicdFile::is_cicd_file(filename) {
                return FileType::CicdFile(path);
            }
            if let Some(language) = Language::from_extension(Path::new(filename).extension().and_then(|e| e.to_str()).unwrap_or("")) {
                return FileType::Regular(language);
            }
        }


}

struct SecondPass {
    languages: Vec<String>,
}

pub struct Parser;

impl Parser {
    pub fn new(path: PathBuf) -> Self {
        println!("Parsing repository: {path:?}");
        Self
    }
}

struct DirectoryIterator {
    queue: VecDeque<PathBuf>,
    root: PathBuf,
}

impl From<PathBuf> for DirectoryIterator {
    fn from(root: PathBuf) -> Self {
        Self {
            queue: VecDeque::from([root.clone()]),
            root,
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
