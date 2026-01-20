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
