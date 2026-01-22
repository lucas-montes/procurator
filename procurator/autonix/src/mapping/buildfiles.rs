use std::path::Path;

use crate::mapping::{ParseError, Parseable};

#[derive(Debug, PartialEq, Eq, Hash, Clone, serde::Serialize, serde::Deserialize)]
pub enum Language {
    Rust,
    JavaScript,
    Python,
    Go,
    Java,
    CSharp,
    Ruby,
    PHP,
    C,
    Bash,
}

impl Language {
    pub fn from_extension(s: &str) -> Option<Self> {
        match s {
            "rs" => Some(Self::Rust),
            "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "py" => Some(Self::Python),
            "go" => Some(Self::Go),
            "java" | "jar" | "class" => Some(Self::Java),
            "cs" | "fs" | "vb" => Some(Self::CSharp),
            "rb" => Some(Self::Ruby),
            "php" => Some(Self::PHP),
            "c" => Some(Self::C),
            "sh" => Some(Self::Bash),
            _ => None,
        }
    }
}

/// Package managers that can be detected from lock files or manifest configuration
/// Used to determine the correct build and dependency installation commands
#[derive(Debug, PartialEq, Eq, Hash, Clone, serde::Serialize, serde::Deserialize)]
pub enum PackageManager {
    // Rust
    Cargo,

    // JavaScript/TypeScript
    Npm,
    Yarn,
    Pnpm,
    Bun,

    // Python
    Pip,
    Pipenv,
    Poetry,
    Pdm,
    Uv,
    Conda,

    // Go
    GoModules,

    // Java/JVM
    Maven,
    Gradle,
    Sbt,

    // .NET/C#
    Nuget,

    // Ruby
    Bundler,

    // PHP
    Composer,
}

impl TryFrom<&str> for PackageManager {
    type Error = ();

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            // Rust
            "cargo" => Ok(Self::Cargo),

            // JavaScript/TypeScript
            "npm" => Ok(Self::Npm),
            "yarn" => Ok(Self::Yarn),
            "pnpm" => Ok(Self::Pnpm),
            "bun" => Ok(Self::Bun),

            // Python
            "pip" => Ok(Self::Pip),
            "pipenv" => Ok(Self::Pipenv),
            "poetry" => Ok(Self::Poetry),
            "pdm" => Ok(Self::Pdm),
            "uv" => Ok(Self::Uv),
            "conda" => Ok(Self::Conda),

            // Go
            "go" | "go-modules" | "gomod" => Ok(Self::GoModules),

            // Java/JVM
            "maven" | "mvn" => Ok(Self::Maven),
            "gradle" => Ok(Self::Gradle),
            "sbt" => Ok(Self::Sbt),

            // .NET/C#
            "nuget" => Ok(Self::Nuget),

            // Ruby
            "bundler" | "bundle" => Ok(Self::Bundler),

            // PHP
            "composer" => Ok(Self::Composer),

            _ => Err(()),
        }
    }
}

/// Build system files that describe how to build a project
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum BuildFile {
    // Make-based
    Makefile,
    GNUmakefile,

    // CMake
    CMakeLists,

    // Meson
    MesonBuild,

    // Bazel
    BazelBuild,
    BazelWorkspace,

    // Ruby
    Rakefile,

    // Ant (Java)
    AntBuildXml,

    // Zig
    BuildZig,

    // Just (command runner)
    Justfile,

    // Task (Go task runner)
    Taskfile,
}

pub struct ParsedBuildFile;

impl Parseable for BuildFile {
    type Output = ParsedBuildFile;

    fn parse(&self, path: &Path) -> Result<Self::Output, ParseError> {
        Ok(ParsedBuildFile)
    }
}

impl TryFrom<&str> for BuildFile {
    type Error = ();

    fn try_from(filename: &str) -> Result<Self, Self::Error> {
        match filename {
            "Makefile" | "makefile" => Ok(Self::Makefile),
            "GNUmakefile" => Ok(Self::GNUmakefile),
            "CMakeLists.txt" => Ok(Self::CMakeLists),
            "meson.build" => Ok(Self::MesonBuild),
            "BUILD" | "BUILD.bazel" => Ok(Self::BazelBuild),
            "WORKSPACE" => Ok(Self::BazelWorkspace),
            "Rakefile" => Ok(Self::Rakefile),
            "build.xml" => Ok(Self::AntBuildXml),
            "build.zig" => Ok(Self::BuildZig),
            "justfile" => Ok(Self::Justfile),
            "Taskfile.yml" | "Taskfile.yaml" => Ok(Self::Taskfile),
            _ => Err(()),
        }
    }
}

/// CI/CD configuration files
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum CiCdFile {
    // GitHub Actions
    GitHubActions,

    // GitLab CI
    GitLabCI,

    // CircleCI
    CircleCI,

    // Travis CI
    TravisCI,

    // Jenkins
    Jenkinsfile,

    // Azure Pipelines
    AzurePipelines,

    // Bitbucket Pipelines
    BitbucketPipelines,

    // Drone CI
    DroneCI,

    // Buildkite
    Buildkite,

    // AppVeyor
    AppVeyor,

    // Wercker
    Wercker,
}

pub struct ParsedCiCdFile;

impl Parseable for CiCdFile {
    type Output = ParsedCiCdFile;

    fn parse(&self, path: &Path) -> Result<Self::Output, ParseError> {
        Ok(ParsedCiCdFile)
    }
}

impl TryFrom<&str> for CiCdFile {
    type Error = ();

    fn try_from(path: &str) -> Result<Self, Self::Error> {
        let is_yaml = path.ends_with(".yml") || path.ends_with(".yaml");
        // GitHub Actions - .github/workflows/*.yml or *.yaml
        if path.contains(".github/workflows/") && is_yaml {
            return Ok(Self::GitHubActions);
        }

        // CircleCI - .circleci/config.yml or any yml in .circleci/
        if path.contains(".circleci/") && is_yaml {
            return Ok(Self::CircleCI);
        }

        // Buildkite - .buildkite/ directory
        if path.contains(".buildkite/") && is_yaml {
            return Ok(Self::Buildkite);
        }

        // Jenkins - contains Jenkinsfile anywhere in path
        if path.contains("Jenkinsfile") {
            return Ok(Self::Jenkinsfile);
        }

        // Simple filename/path suffix matches
        match path {
            // Exact filename matches
            ".gitlab-ci.yml" | ".gitlab-ci.yaml" => Ok(Self::GitLabCI),
            ".travis.yml" => Ok(Self::TravisCI),
            "Jenkinsfile" => Ok(Self::Jenkinsfile),
            "azure-pipelines.yml" | "azure-pipelines.yaml" => Ok(Self::AzurePipelines),
            "bitbucket-pipelines.yml" => Ok(Self::BitbucketPipelines),
            ".drone.yml" | ".drone.yaml" => Ok(Self::DroneCI),
            "buildkite.yml" | "buildkite.yaml" => Ok(Self::Buildkite),
            "appveyor.yml" | ".appveyor.yml" => Ok(Self::AppVeyor),
            "wercker.yml" => Ok(Self::Wercker),

            // Path suffix matches
            _ if path.ends_with(".gitlab-ci.yml") || path.ends_with(".gitlab-ci.yaml") => {
                Ok(Self::GitLabCI)
            }
            _ if path.ends_with(".travis.yml") => Ok(Self::TravisCI),
            _ if path.ends_with("azure-pipelines.yml")
                || path.ends_with("azure-pipelines.yaml") =>
            {
                Ok(Self::AzurePipelines)
            }
            _ if path.ends_with("bitbucket-pipelines.yml") => Ok(Self::BitbucketPipelines),
            _ if path.ends_with(".drone.yml") || path.ends_with(".drone.yaml") => Ok(Self::DroneCI),
            _ if path.ends_with("buildkite.yml") || path.ends_with("buildkite.yaml") => {
                Ok(Self::Buildkite)
            }
            _ if path.ends_with("appveyor.yml") || path.ends_with(".appveyor.yml") => {
                Ok(Self::AppVeyor)
            }
            _ if path.ends_with("wercker.yml") => Ok(Self::Wercker),
            _ if path.ends_with("Jenkinsfile") => Ok(Self::Jenkinsfile),

            _ => Err(()),
        }
    }
}
