
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
