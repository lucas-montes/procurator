// Parse the repository and discover its structure

use std::{
    collections::HashMap,
    ffi::OsStr,
    path::{Path, PathBuf},
};

use crate::mapping::{Language, LockFile, ManifestFile};

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

/// A tree representation of the scanned repository
#[derive(Debug, PartialEq)]
pub struct Scan {
    root: ScanNode,
}

impl From<PathBuf> for Scan {
    fn from(root: PathBuf) -> Self {
        Self { root: root.into() }
    }
}

/// A node in the scan tree representing a directory
#[derive(Debug, PartialEq)]
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

#[derive(Debug, Default, PartialEq)]
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

#[derive(Debug, PartialEq)]
struct FilePath<T> {
    kind: T,
    path: PathBuf,
}

#[derive(Debug, PartialEq)]
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

#[derive(Debug, PartialEq)]
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


#[cfg(test)]
mod tests {
    use super::*;

    fn test_features_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("features")
    }

    #[test]
    fn test_rust_standalone() {
        let path = test_features_path().join("rust_standalone");
        let scan = Scan::from(path.clone());

        let expected = Scan {
            root: ScanNode {
                path: path.clone(),
                files: DirectoryScan {
                    manifest_files: vec![FilePath {
                        kind: ManifestFile::CargoToml,
                        path: path.join("Cargo.toml"),
                    }],
                    lockfiles: vec![],
                    buildfiles: vec![],
                    cicd_files: vec![],
                    file_per_language: HashMap::new(),
                },
                children: vec![ScanNode {
                    path: path.join("src"),
                    files: DirectoryScan {
                        manifest_files: vec![],
                        lockfiles: vec![],
                        buildfiles: vec![],
                        cicd_files: vec![],
                        file_per_language: HashMap::from([(Language::Rust, 1)]),
                    },
                    children: vec![],
                }],
            },
        };

        assert_eq!(scan, expected);
    }

    #[test]
    fn test_rust_workspace() {
        let path = test_features_path().join("rust_workspace");
        let scan = Scan::from(path.clone());

        let crates_path = path.join("crates");
        let crate_a_path = crates_path.join("crate_a");
        let crate_b_path = crates_path.join("crate_b");

        let expected = Scan {
            root: ScanNode {
                path: path.clone(),
                files: DirectoryScan {
                    manifest_files: vec![FilePath {
                        kind: ManifestFile::CargoToml,
                        path: path.join("Cargo.toml"),
                    }],
                    lockfiles: vec![],
                    buildfiles: vec![],
                    cicd_files: vec![],
                    file_per_language: HashMap::new(),
                },
                children: vec![ScanNode {
                    path: crates_path.clone(),
                    files: DirectoryScan {
                        manifest_files: vec![],
                        lockfiles: vec![],
                        buildfiles: vec![],
                        cicd_files: vec![],
                        file_per_language: HashMap::new(),
                    },
                    children: vec![
                        ScanNode {
                            path: crate_b_path.clone(),
                            files: DirectoryScan {
                                manifest_files: vec![FilePath {
                                    kind: ManifestFile::CargoToml,
                                    path: crate_b_path.join("Cargo.toml"),
                                }],
                                lockfiles: vec![],
                                buildfiles: vec![],
                                cicd_files: vec![],
                                file_per_language: HashMap::new(),
                            },
                            children: vec![ScanNode {
                                path: crate_b_path.join("src"),
                                files: DirectoryScan {
                                    manifest_files: vec![],
                                    lockfiles: vec![],
                                    buildfiles: vec![],
                                    cicd_files: vec![],
                                    file_per_language: HashMap::from([(Language::Rust, 1)]),
                                },
                                children: vec![],
                            }],
                        },
                        ScanNode {
                            path: crate_a_path.clone(),
                            files: DirectoryScan {
                                manifest_files: vec![FilePath {
                                    kind: ManifestFile::CargoToml,
                                    path: crate_a_path.join("Cargo.toml"),
                                }],
                                lockfiles: vec![],
                                buildfiles: vec![],
                                cicd_files: vec![],
                                file_per_language: HashMap::new(),
                            },
                            children: vec![ScanNode {
                                path: crate_a_path.join("src"),
                                files: DirectoryScan {
                                    manifest_files: vec![],
                                    lockfiles: vec![],
                                    buildfiles: vec![],
                                    cicd_files: vec![],
                                    file_per_language: HashMap::from([(Language::Rust, 1)]),
                                },
                                children: vec![],
                            }],
                        },
                    ],
                }],
            },
        };

        assert_eq!(scan, expected);
    }

    #[test]
    fn test_node_monorepo() {
        let path = test_features_path().join("node_monorepo");
        let scan = Scan::from(path.clone());

        let packages_path = path.join("packages");
        let shared_path = packages_path.join("shared");
        let web_path = packages_path.join("web");
        let web_src_path = web_path.join("src");

        let expected = Scan {
            root: ScanNode {
                path: path.clone(),
                files: DirectoryScan {
                    manifest_files: vec![FilePath {
                        kind: ManifestFile::PackageJson,
                        path: path.join("package.json"),
                    }],
                    lockfiles: vec![FilePath {
                        kind: LockFile::PackageLockJson,
                        path: path.join("package-lock.json"),
                    }],
                    buildfiles: vec![],
                    cicd_files: vec![],
                    file_per_language: HashMap::new(),
                },
                children: vec![ScanNode {
                    path: packages_path.clone(),
                    files: DirectoryScan {
                        manifest_files: vec![],
                        lockfiles: vec![],
                        buildfiles: vec![],
                        cicd_files: vec![],
                        file_per_language: HashMap::new(),
                    },
                    children: vec![
                        ScanNode {
                            path: shared_path.clone(),
                            files: DirectoryScan {
                                manifest_files: vec![FilePath {
                                    kind: ManifestFile::PackageJson,
                                    path: shared_path.join("package.json"),
                                }],
                                lockfiles: vec![],
                                buildfiles: vec![],
                                cicd_files: vec![],
                                file_per_language: HashMap::from([(Language::JavaScript, 1)]),
                            },
                            children: vec![],
                        },
                        ScanNode {
                            path: web_path.clone(),
                            files: DirectoryScan {
                                manifest_files: vec![FilePath {
                                    kind: ManifestFile::PackageJson,
                                    path: web_path.join("package.json"),
                                }],
                                lockfiles: vec![],
                                buildfiles: vec![],
                                cicd_files: vec![],
                                file_per_language: HashMap::new(),
                            },
                            children: vec![ScanNode {
                                path: web_src_path.clone(),
                                files: DirectoryScan {
                                    manifest_files: vec![],
                                    lockfiles: vec![],
                                    buildfiles: vec![],
                                    cicd_files: vec![],
                                    file_per_language: HashMap::from([(Language::JavaScript, 1)]),
                                },
                                children: vec![],
                            }],
                        },
                    ],
                }],
            },
        };

        assert_eq!(scan, expected);
    }

    #[test]
    fn test_python_project() {
        let path = test_features_path().join("python_project");
        let scan = Scan::from(path.clone());

        let src_path = path.join("src");

        let expected = Scan {
            root: ScanNode {
                path: path.clone(),
                files: DirectoryScan {
                    manifest_files: vec![FilePath {
                        kind: ManifestFile::PyprojectToml,
                        path: path.join("pyproject.toml"),
                    }],
                    lockfiles: vec![FilePath {
                        kind: LockFile::PoetryLock,
                        path: path.join("poetry.lock"),
                    }],
                    buildfiles: vec![],
                    cicd_files: vec![],
                    file_per_language: HashMap::new(),
                },
                children: vec![ScanNode {
                    path: src_path.clone(),
                    files: DirectoryScan {
                        manifest_files: vec![],
                        lockfiles: vec![],
                        buildfiles: vec![],
                        cicd_files: vec![],
                        file_per_language: HashMap::from([(Language::Python, 1)]),
                    },
                    children: vec![],
                }],
            },
        };

        assert_eq!(scan, expected);
    }

    #[test]
    fn test_go_standalone() {
        let path = test_features_path().join("go_standalone");
        let scan = Scan::from(path.clone());

        let expected = Scan {
            root: ScanNode {
                path: path.clone(),
                files: DirectoryScan {
                    manifest_files: vec![FilePath {
                        kind: ManifestFile::GoMod,
                        path: path.join("go.mod"),
                    }],
                    lockfiles: vec![FilePath {
                        kind: LockFile::GoSum,
                        path: path.join("go.sum"),
                    }],
                    buildfiles: vec![],
                    cicd_files: vec![],
                    file_per_language: HashMap::from([(Language::Go, 1)]),
                },
                children: vec![],
            },
        };

        assert_eq!(scan, expected);
    }

    #[test]
    fn test_go_workspace() {
        let path = test_features_path().join("go_workspace");
        let scan = Scan::from(path.clone());

        let backend_path = path.join("backend");
        let shared_path = path.join("shared");

        let expected = Scan {
            root: ScanNode {
                path: path.clone(),
                files: DirectoryScan {
                    manifest_files: vec![],
                    lockfiles: vec![],
                    buildfiles: vec![],
                    cicd_files: vec![],
                    file_per_language: HashMap::new(),
                },
                children: vec![
                    ScanNode {
                        path: shared_path.clone(),
                        files: DirectoryScan {
                            manifest_files: vec![FilePath {
                                kind: ManifestFile::GoMod,
                                path: shared_path.join("go.mod"),
                            }],
                            lockfiles: vec![],
                            buildfiles: vec![],
                            cicd_files: vec![],
                            file_per_language: HashMap::from([(Language::Go, 1)]),
                        },
                        children: vec![],
                    },
                    ScanNode {
                        path: backend_path.clone(),
                        files: DirectoryScan {
                            manifest_files: vec![FilePath {
                                kind: ManifestFile::GoMod,
                                path: backend_path.join("go.mod"),
                            }],
                            lockfiles: vec![],
                            buildfiles: vec![],
                            cicd_files: vec![],
                            file_per_language: HashMap::from([(Language::Go, 1)]),
                        },
                        children: vec![],
                    },
                ],
            },
        };

        assert_eq!(scan, expected);
    }

    #[test]
    fn test_mixed_language() {
        let path = test_features_path().join("mixed_language");
        let scan = Scan::from(path.clone());

        let src_path = path.join("src");

        let expected = Scan {
            root: ScanNode {
                path: path.clone(),
                files: DirectoryScan {
                    manifest_files: vec![
                        FilePath {
                            kind: ManifestFile::PackageJson,
                            path: path.join("package.json"),
                        },
                        FilePath {
                            kind: ManifestFile::CargoToml,
                            path: path.join("Cargo.toml"),
                        },
                    ],
                    lockfiles: vec![],
                    buildfiles: vec![],
                    cicd_files: vec![],
                    file_per_language: HashMap::from([(Language::JavaScript, 1)]),
                },
                children: vec![ScanNode {
                    path: src_path.clone(),
                    files: DirectoryScan {
                        manifest_files: vec![],
                        lockfiles: vec![],
                        buildfiles: vec![],
                        cicd_files: vec![],
                        file_per_language: HashMap::from([(Language::Rust, 1)]),
                    },
                    children: vec![],
                }],
            },
        };

        assert_eq!(scan, expected);
    }

    #[test]
    fn test_multi_language_monorepo() {
        let path = test_features_path().join("multi_language_monorepo");
        let scan = Scan::from(path.clone());

        let backend_path = path.join("backend");
        let backend_src_path = backend_path.join("src");
        let frontend_path = path.join("frontend");
        let frontend_src_path = frontend_path.join("src");
        let scripts_path = path.join("scripts");

        let expected = Scan {
            root: ScanNode {
                path: path.clone(),
                files: DirectoryScan {
                    manifest_files: vec![],
                    lockfiles: vec![],
                    buildfiles: vec![],
                    cicd_files: vec![],
                    file_per_language: HashMap::new(),
                },
                children: vec![
                    ScanNode {
                        path: scripts_path.clone(),
                        files: DirectoryScan {
                            manifest_files: vec![FilePath {
                                kind: ManifestFile::RequirementsTxt,
                                path: scripts_path.join("requirements.txt"),
                            }],
                            lockfiles: vec![],
                            buildfiles: vec![],
                            cicd_files: vec![],
                            file_per_language: HashMap::from([(Language::Python, 1)]),
                        },
                        children: vec![],
                    },
                    ScanNode {
                        path: backend_path.clone(),
                        files: DirectoryScan {
                            manifest_files: vec![FilePath {
                                kind: ManifestFile::CargoToml,
                                path: backend_path.join("Cargo.toml"),
                            }],
                            lockfiles: vec![],
                            buildfiles: vec![],
                            cicd_files: vec![],
                            file_per_language: HashMap::new(),
                        },
                        children: vec![ScanNode {
                            path: backend_src_path.clone(),
                            files: DirectoryScan {
                                manifest_files: vec![],
                                lockfiles: vec![],
                                buildfiles: vec![],
                                cicd_files: vec![],
                                file_per_language: HashMap::from([(Language::Rust, 1)]),
                            },
                            children: vec![],
                        }],
                    },
                    ScanNode {
                        path: frontend_path.clone(),
                        files: DirectoryScan {
                            manifest_files: vec![FilePath {
                                kind: ManifestFile::PackageJson,
                                path: frontend_path.join("package.json"),
                            }],
                            lockfiles: vec![FilePath {
                                kind: LockFile::PackageLockJson,
                                path: frontend_path.join("package-lock.json"),
                            }],
                            buildfiles: vec![],
                            cicd_files: vec![],
                            file_per_language: HashMap::new(),
                        },
                        children: vec![ScanNode {
                            path: frontend_src_path.clone(),
                            files: DirectoryScan {
                                manifest_files: vec![],
                                lockfiles: vec![],
                                buildfiles: vec![],
                                cicd_files: vec![],
                                file_per_language: HashMap::from([(Language::JavaScript, 1)]),
                            },
                            children: vec![],
                        }],
                    },
                ],
            },
        };

        assert_eq!(scan, expected);
    }

    #[test]
    fn test_nested_independent() {
        let path = test_features_path().join("nested_independent");
        let scan = Scan::from(path.clone());

        let project_path = path.join("project");
        let tools_path = project_path.join("tools");
        let converter_path = tools_path.join("converter");

        let expected = Scan {
            root: ScanNode {
                path: path.clone(),
                files: DirectoryScan {
                    manifest_files: vec![],
                    lockfiles: vec![],
                    buildfiles: vec![],
                    cicd_files: vec![],
                    file_per_language: HashMap::new(),
                },
                children: vec![ScanNode {
                    path: project_path.clone(),
                    files: DirectoryScan {
                        manifest_files: vec![FilePath {
                            kind: ManifestFile::GoMod,
                            path: project_path.join("go.mod"),
                        }],
                        lockfiles: vec![],
                        buildfiles: vec![],
                        cicd_files: vec![],
                        file_per_language: HashMap::from([(Language::Go, 1)]),
                    },
                    children: vec![ScanNode {
                        path: tools_path.clone(),
                        files: DirectoryScan {
                            manifest_files: vec![],
                            lockfiles: vec![],
                            buildfiles: vec![],
                            cicd_files: vec![],
                            file_per_language: HashMap::new(),
                        },
                        children: vec![ScanNode {
                            path: converter_path.clone(),
                            files: DirectoryScan {
                                manifest_files: vec![FilePath {
                                    kind: ManifestFile::PackageJson,
                                    path: converter_path.join("package.json"),
                                }],
                                lockfiles: vec![],
                                buildfiles: vec![],
                                cicd_files: vec![],
                                file_per_language: HashMap::from([(Language::JavaScript, 1)]),
                            },
                            children: vec![],
                        }],
                    }],
                }],
            },
        };

        assert_eq!(scan, expected);
    }

    #[test]
    fn test_nested_workspaces() {
        let path = test_features_path().join("nested_workspaces");
        let scan = Scan::from(path.clone());

        let crate_a_path = path.join("crate_a");
        let crate_a_src_path = crate_a_path.join("src");
        let mega_crate_path = path.join("mega_crate");
        let sub_a_path = mega_crate_path.join("sub_a");
        let sub_a_src_path = sub_a_path.join("src");
        let sub_b_path = mega_crate_path.join("sub_b");
        let sub_b_src_path = sub_b_path.join("src");

        let expected = Scan {
            root: ScanNode {
                path: path.clone(),
                files: DirectoryScan {
                    manifest_files: vec![FilePath {
                        kind: ManifestFile::CargoToml,
                        path: path.join("Cargo.toml"),
                    }],
                    lockfiles: vec![],
                    buildfiles: vec![],
                    cicd_files: vec![],
                    file_per_language: HashMap::new(),
                },
                children: vec![
                    ScanNode {
                        path: crate_a_path.clone(),
                        files: DirectoryScan {
                            manifest_files: vec![FilePath {
                                kind: ManifestFile::CargoToml,
                                path: crate_a_path.join("Cargo.toml"),
                            }],
                            lockfiles: vec![],
                            buildfiles: vec![],
                            cicd_files: vec![],
                            file_per_language: HashMap::new(),
                        },
                        children: vec![ScanNode {
                            path: crate_a_src_path.clone(),
                            files: DirectoryScan {
                                manifest_files: vec![],
                                lockfiles: vec![],
                                buildfiles: vec![],
                                cicd_files: vec![],
                                file_per_language: HashMap::from([(Language::Rust, 1)]),
                            },
                            children: vec![],
                        }],
                    },
                    ScanNode {
                        path: mega_crate_path.clone(),
                        files: DirectoryScan {
                            manifest_files: vec![FilePath {
                                kind: ManifestFile::CargoToml,
                                path: mega_crate_path.join("Cargo.toml"),
                            }],
                            lockfiles: vec![],
                            buildfiles: vec![],
                            cicd_files: vec![],
                            file_per_language: HashMap::new(),
                        },
                        children: vec![
                            ScanNode {
                                path: sub_a_path.clone(),
                                files: DirectoryScan {
                                    manifest_files: vec![FilePath {
                                        kind: ManifestFile::CargoToml,
                                        path: sub_a_path.join("Cargo.toml"),
                                    }],
                                    lockfiles: vec![],
                                    buildfiles: vec![],
                                    cicd_files: vec![],
                                    file_per_language: HashMap::new(),
                                },
                                children: vec![ScanNode {
                                    path: sub_a_src_path.clone(),
                                    files: DirectoryScan {
                                        manifest_files: vec![],
                                        lockfiles: vec![],
                                        buildfiles: vec![],
                                        cicd_files: vec![],
                                        file_per_language: HashMap::from([(Language::Rust, 1)]),
                                    },
                                    children: vec![],
                                }],
                            },
                            ScanNode {
                                path: sub_b_path.clone(),
                                files: DirectoryScan {
                                    manifest_files: vec![FilePath {
                                        kind: ManifestFile::CargoToml,
                                        path: sub_b_path.join("Cargo.toml"),
                                    }],
                                    lockfiles: vec![],
                                    buildfiles: vec![],
                                    cicd_files: vec![],
                                    file_per_language: HashMap::new(),
                                },
                                children: vec![ScanNode {
                                    path: sub_b_src_path.clone(),
                                    files: DirectoryScan {
                                        manifest_files: vec![],
                                        lockfiles: vec![],
                                        buildfiles: vec![],
                                        cicd_files: vec![],
                                        file_per_language: HashMap::from([(Language::Rust, 1)]),
                                    },
                                    children: vec![],
                                }],
                            },
                        ],
                    },
                ],
            },
        };

        assert_eq!(scan, expected);
    }
}
