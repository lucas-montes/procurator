// Enums mapping from files to their respective types in the Autonix system.

/// Manifest files declare dependencies and project metadata
/// These are the primary files that tell us what a project needs
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum ManifestFile {
    // Rust
    CargoToml,

    // JavaScript/TypeScript
    PackageJson,

    // Python - Primary manifest files
    PyprojectToml,   // Modern Python projects (PEP 621)
    SetupPy,         // Legacy setuptools
    SetupCfg,        // Alternative setuptools config
    RequirementsTxt, // pip requirements (includes common misspellings)
    Pipfile,         // pipenv

    // Python - Additional project indicators
    PythonVersion,  // .python-version - runtime version specifier
    RuntimeTxt,     // runtime.txt - Heroku/platform runtime specifier
    CondaYaml,      // conda.yaml - Conda environment
    EnvironmentYml, // environment.yml - Conda environment alternative name

    // Go
    GoMod,

    // Java/JVM
    PomXml,         // Maven
    BuildGradle,    // Gradle (Groovy)
    BuildGradleKts, // Gradle (Kotlin)
    BuildSbt,       // Scala SBT

    // .NET/C#
    Csproj, // C# project file
    Fsproj, // F# project file
    Sln,    // Visual Studio solution

    // Ruby
    Gemfile,

    // PHP
    ComposerJson,
}

/// Try to parse a filename into a ManifestFile enum
/// Only matches exact filenames, handles both with and without path
impl TryFrom<&str> for ManifestFile {
    type Error = ();

    fn try_from(filename: &str) -> Result<Self, Self::Error> {
        match filename {
            // Rust
            "Cargo.toml" => Ok(Self::CargoToml),

            // JavaScript/TypeScript
            "package.json" => Ok(Self::PackageJson),

            // Python
            "pyproject.toml" => Ok(Self::PyprojectToml),
            "setup.py" => Ok(Self::SetupPy),
            "setup.cfg" => Ok(Self::SetupCfg),
            "requirements.txt" => Ok(Self::RequirementsTxt),
            "Pipfile" => Ok(Self::Pipfile),
            ".python-version" => Ok(Self::PythonVersion),
            "runtime.txt" => Ok(Self::RuntimeTxt),
            "conda.yaml" | "conda.yml" => Ok(Self::CondaYaml),
            "environment.yml" | "environment.yaml" => Ok(Self::EnvironmentYml),
            // Python requirements.txt misspellings - all map to RequirementsTxt
            "requeriments.txt"
            | "requirement.txt"
            | "requirements"
            | "requirements.text"
            | "Requirements.txt"
            | "requirements.txt.txt"
            | "requirments.txt" => Ok(Self::RequirementsTxt),

            // Go
            "go.mod" => Ok(Self::GoMod),

            // Java/JVM
            "pom.xml" => Ok(Self::PomXml),
            "build.gradle" => Ok(Self::BuildGradle),
            "build.gradle.kts" => Ok(Self::BuildGradleKts),
            "build.sbt" => Ok(Self::BuildSbt),

            // Ruby
            "Gemfile" => Ok(Self::Gemfile),

            // PHP
            "composer.json" => Ok(Self::ComposerJson),

            // .NET/C# - extension-based project files
            _ if filename.ends_with(".csproj") => Ok(Self::Csproj),
            _ if filename.ends_with(".fsproj") => Ok(Self::Fsproj),
            _ if filename.ends_with(".sln") => Ok(Self::Sln),

            _ => Err(()),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum LockFile {
    // Rust
    CargoLock,

    // JavaScript/TypeScript/JavaScript
    PackageLockJson, // npm
    YarnLock,        // yarn classic (v1)
    PnpmLockYaml,    // pnpm
    BunLockb,        // bun (binary format)

    // Python
    PipfileLock, // pipenv
    PoetryLock,  // poetry
    PdmLock,     // pdm
    UvLock,      // uv (new fast Python package manager)

    // Go
    GoSum, // go modules checksums

    // Java/JVM
    GradleLockfile,

    // .NET/C#
    PackagesLockJson, // NuGet

    // Ruby
    GemfileLock,

    // PHP
    ComposerLock,
}

impl TryFrom<&str> for LockFile {
    type Error = ();

    fn try_from(filename: &str) -> Result<Self, Self::Error> {
        match filename {
            // Rust
            "Cargo.lock" => Ok(Self::CargoLock),

            // JavaScript/TypeScript
            "package-lock.json" => Ok(Self::PackageLockJson),
            "yarn.lock" => Ok(Self::YarnLock),
            "pnpm-lock.yaml" => Ok(Self::PnpmLockYaml),
            "bun.lockb" => Ok(Self::BunLockb),

            // Python
            "Pipfile.lock" => Ok(Self::PipfileLock),
            "poetry.lock" => Ok(Self::PoetryLock),
            "pdm.lock" => Ok(Self::PdmLock),
            "uv.lock" => Ok(Self::UvLock),

            // Go
            "go.sum" => Ok(Self::GoSum),

            // Java/JVM
            "gradle.lockfile" => Ok(Self::GradleLockfile),

            // .NET/C#
            "packages.lock.json" => Ok(Self::PackagesLockJson),

            // Ruby
            "Gemfile.lock" => Ok(Self::GemfileLock),

            // PHP
            "composer.lock" => Ok(Self::ComposerLock),

            _ => Err(()),
        }
    }
}

impl From<&ManifestFile> for Language {
    fn from(file: &ManifestFile) -> Self {
        match file {
            // Rust
            ManifestFile::CargoToml => Self::Rust,

            // JavaScript/TypeScript
            ManifestFile::PackageJson => Self::JavaScript,

            // Python
            ManifestFile::PyprojectToml
            | ManifestFile::SetupPy
            | ManifestFile::SetupCfg
            | ManifestFile::RequirementsTxt
            | ManifestFile::Pipfile
            | ManifestFile::PythonVersion
            | ManifestFile::RuntimeTxt
            | ManifestFile::CondaYaml
            | ManifestFile::EnvironmentYml => Self::Python,

            // Go
            ManifestFile::GoMod => Self::Go,

            // Java/JVM
            ManifestFile::PomXml
            | ManifestFile::BuildGradle
            | ManifestFile::BuildGradleKts
            | ManifestFile::BuildSbt => Self::Java,

            // .NET/C#
            ManifestFile::Csproj | ManifestFile::Fsproj | ManifestFile::Sln => Self::CSharp,

            // Ruby
            ManifestFile::Gemfile => Self::Ruby,

            // PHP
            ManifestFile::ComposerJson => Self::PHP,
        }
    }
}

impl From<&LockFile> for Language {
    fn from(file: &LockFile) -> Self {
        match file {
            // Rust
            LockFile::CargoLock => Self::Rust,

            // JavaScript/TypeScript
            LockFile::PackageLockJson
            | LockFile::YarnLock
            | LockFile::PnpmLockYaml
            | LockFile::BunLockb => Self::JavaScript,

            // Python
            LockFile::PipfileLock | LockFile::PoetryLock | LockFile::PdmLock | LockFile::UvLock => {
                Self::Python
            }

            // Go
            LockFile::GoSum => Self::Go,

            // Java/JVM
            LockFile::GradleLockfile => Self::Java,

            // .NET/C#
            LockFile::PackagesLockJson => Self::CSharp,

            // Ruby
            LockFile::GemfileLock => Self::Ruby,

            // PHP
            LockFile::ComposerLock => Self::PHP,
        }
    }
}

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

impl From<&PackageManager> for Language {
    fn from(pm: &PackageManager) -> Self {
        match pm {
            PackageManager::Cargo => Language::Rust,
            PackageManager::Npm
            | PackageManager::Yarn
            | PackageManager::Pnpm
            | PackageManager::Bun => Language::JavaScript,
            PackageManager::Pip
            | PackageManager::Pipenv
            | PackageManager::Poetry
            | PackageManager::Pdm
            | PackageManager::Uv
            | PackageManager::Conda => Language::Python,
            PackageManager::GoModules => Language::Go,
            PackageManager::Maven | PackageManager::Gradle | PackageManager::Sbt => Language::Java,
            PackageManager::Nuget => Language::CSharp,
            PackageManager::Bundler => Language::Ruby,
            PackageManager::Composer => Language::PHP,
        }
    }
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

/// Infer package manager from a lock file
impl From<&LockFile> for PackageManager {
    fn from(file: &LockFile) -> Self {
        match file {
            // Rust
            LockFile::CargoLock => Self::Cargo,

            // JavaScript/TypeScript
            LockFile::PackageLockJson => Self::Npm,
            LockFile::YarnLock => Self::Yarn,
            LockFile::PnpmLockYaml => Self::Pnpm,
            LockFile::BunLockb => Self::Bun,

            // Python
            LockFile::PipfileLock => Self::Pipenv,
            LockFile::PoetryLock => Self::Poetry,
            LockFile::PdmLock => Self::Pdm,
            LockFile::UvLock => Self::Uv,

            // Go
            LockFile::GoSum => Self::GoModules,

            // Java/JVM
            LockFile::GradleLockfile => Self::Gradle,

            // .NET/C#
            LockFile::PackagesLockJson => Self::Nuget,

            // Ruby
            LockFile::GemfileLock => Self::Bundler,

            // PHP
            LockFile::ComposerLock => Self::Composer,
        }
    }
}

/// Infer package manager from a manifest file
impl TryFrom<&ManifestFile> for PackageManager {
    type Error = ();

    fn try_from(file: &ManifestFile) -> Result<Self, Self::Error> {
        match file {
            // Rust
            ManifestFile::CargoToml => Ok(Self::Cargo),

            // JavaScript/TypeScript
            ManifestFile::PackageJson => Err(()), // Need to check packageManager field

            // Python
            ManifestFile::PyprojectToml => Err(()), // Could be poetry, pdm, or pip
            ManifestFile::SetupPy | ManifestFile::SetupCfg | ManifestFile::RequirementsTxt => {
                Ok(Self::Pip)
            }
            ManifestFile::Pipfile => Ok(Self::Pipenv),
            ManifestFile::PythonVersion | ManifestFile::RuntimeTxt => Err(()), // Just version specifiers
            ManifestFile::CondaYaml | ManifestFile::EnvironmentYml => Ok(Self::Conda),

            // Go
            ManifestFile::GoMod => Ok(Self::GoModules),

            // Java/JVM
            ManifestFile::PomXml => Ok(Self::Maven),
            ManifestFile::BuildGradle | ManifestFile::BuildGradleKts => Ok(Self::Gradle),
            ManifestFile::BuildSbt => Ok(Self::Sbt),

            // .NET/C#
            ManifestFile::Csproj | ManifestFile::Fsproj | ManifestFile::Sln => Ok(Self::Nuget),

            // Ruby
            ManifestFile::Gemfile => Ok(Self::Bundler),

            // PHP
            ManifestFile::ComposerJson => Ok(Self::Composer),
        }
    }
}
