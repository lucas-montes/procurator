
use std::fmt;

use serde::Serialize;

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Serialize)]
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

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Language::Rust => write!(f, "rust"),
            Language::JavaScript => write!(f, "javascript"),
            Language::Python => write!(f, "python3"),
            Language::Go => write!(f, "go"),
            Language::Ruby => write!(f, "ruby"),
            Language::Java => write!(f, "jdk"),
            Language::CSharp => write!(f, "dotnet-sdk"),
            Language::C => write!(f, "c"),
            Language::PHP => write!(f, "php"),
            Language::Bash => write!(f, "bash"),
        }
    }
}

/// Package managers that can be detected from lock files or manifest configuration
/// Used to determine the correct build and dependency installation commands
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Serialize)]
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

impl fmt::Display for PackageManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageManager::Cargo => write!(f, "cargo"),
            PackageManager::Npm => write!(f, "npm"),
            PackageManager::Yarn => write!(f, "yarn"),
            PackageManager::Pnpm => write!(f, "pnpm"),
            PackageManager::Bun => write!(f, "bun"),
            PackageManager::Poetry => write!(f, "poetry"),
            PackageManager::Pip => write!(f, "pip"),
            PackageManager::Pipenv => write!(f, "pipenv"),
            PackageManager::Pdm => write!(f, "pdm"),
            PackageManager::Uv => write!(f, "uv"),
            PackageManager::Conda => write!(f, "conda"),
            PackageManager::GoModules => write!(f, "go"),
            PackageManager::Maven => write!(f, "maven"),
            PackageManager::Gradle => write!(f, "gradle"),
            PackageManager::Sbt => write!(f, "sbt"),
            PackageManager::Nuget => write!(f, "nuget"),
            PackageManager::Bundler => write!(f, "bundler"),
            PackageManager::Composer => write!(f, "composer"),
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
