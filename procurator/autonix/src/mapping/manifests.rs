use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

use crate::mapping::{ParseError, Parseable};

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

/// Common parsed manifest result
#[derive(Debug)]
pub struct ParsedManifest {
    pub names: Vec<String>,
    pub version: Option<String>,
    pub workspace_members: Vec<String>,
    pub metadata: ManifestMetadata,
    pub entry_points: Vec<String>,
    pub scripts: HashMap<String, String>,
    pub toolchain_version: Option<String>,
}

#[derive(Debug, Default)]
pub struct ManifestMetadata {
    pub description: Option<String>,
    pub authors: Vec<String>,
    pub license: Option<String>,
}

// Cargo.toml structures
#[derive(Deserialize)]
struct CargoToml {
    package: Option<CargoPackage>,
    workspace: Option<CargoWorkspace>,
    #[serde(rename = "bin")]
    bins: Option<Vec<CargoBin>>,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: String,
    version: String,
    edition: Option<String>,
    description: Option<String>,
    authors: Option<Vec<String>>,
    license: Option<String>,
}

#[derive(Deserialize)]
struct CargoWorkspace {
    members: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct CargoBin {
    name: String,
}

// package.json structures
#[derive(Deserialize)]
struct PackageJson {
    name: String,
    version: Option<String>,
    description: Option<String>,
    author: Option<String>,
    license: Option<String>,
    workspaces: Option<PackageJsonWorkspaces>,
    bin: Option<PackageJsonBin>,
    scripts: Option<HashMap<String, String>>,
    engines: Option<PackageJsonEngines>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PackageJsonWorkspaces {
    Array(Vec<String>),
    Object { packages: Vec<String> },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PackageJsonBin {
    Single(String),
    Multiple(HashMap<String, String>),
}

#[derive(Deserialize)]
struct PackageJsonEngines {
    node: Option<String>,
}

// pyproject.toml structures
#[derive(Deserialize)]
struct PyprojectToml {
    project: Option<PythonProject>,
}

#[derive(Deserialize)]
struct PythonProject {
    name: String,
    version: Option<String>,
    description: Option<String>,
    authors: Option<Vec<PythonAuthor>>,
    license: Option<PythonLicense>,
    #[serde(rename = "requires-python")]
    requires_python: Option<String>,
    scripts: Option<HashMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PythonAuthor {
    Simple(String),
    Detailed {
        name: String,
        #[allow(dead_code)]
        email: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PythonLicense {
    Simple(String),
    Detailed {
        text: Option<String>,
        #[allow(dead_code)]
        file: Option<String>,
    },
}

impl Parseable for ManifestFile {
    type Output = ParsedManifest;

    fn parse(&self, path: &Path) -> Result<Self::Output, ParseError> {
        let content = std::fs::read_to_string(path)?;

        match self {
            Self::CargoToml => parse_cargo_toml(&content),
            Self::PackageJson => parse_package_json(&content),
            Self::PyprojectToml => parse_pyproject_toml(&content),
            Self::GoMod => parse_go_mod(&content),

            // Version files (simple single-line reads)
            Self::PythonVersion | Self::RuntimeTxt => Ok(ParsedManifest {
                names: vec![],
                version: None,
                workspace_members: vec![],
                metadata: ManifestMetadata::default(),
                entry_points: vec![],
                scripts: HashMap::new(),
                toolchain_version: Some(content.trim().to_string()),
            }),

            // Not yet implemented - return empty manifest
            Self::PomXml
            | Self::BuildGradle
            | Self::BuildGradleKts
            | Self::ComposerJson
            | Self::Gemfile
            | Self::RequirementsTxt
            | Self::Pipfile
            | Self::CondaYaml
            | Self::EnvironmentYml
            | Self::SetupPy
            | Self::SetupCfg
            | Self::Csproj
            | Self::Fsproj
            | Self::Sln
            | Self::BuildSbt => Ok(ParsedManifest::default()),
        }
    }
}

impl Default for ParsedManifest {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            version: None,
            workspace_members: Vec::new(),
            metadata: ManifestMetadata::default(),
            entry_points: Vec::new(),
            scripts: HashMap::new(),
            toolchain_version: None,
        }
    }
}

fn parse_cargo_toml(content: &str) -> Result<ParsedManifest, ParseError> {
    let cargo: CargoToml = toml::from_str(content)?;
    Ok(cargo.into())
}

impl From<CargoToml> for ParsedManifest {
    fn from(cargo: CargoToml) -> Self {
        let mut result = ParsedManifest::default();

        if let Some(pkg) = cargo.package {
            result.names.push(pkg.name);
            result.version = Some(pkg.version);
            result.metadata.description = pkg.description;
            result.metadata.authors = pkg.authors.unwrap_or_default();
            result.metadata.license = pkg.license;
            result.toolchain_version = pkg.edition;
        }

        if let Some(ws) = cargo.workspace {
            result.workspace_members = ws.members.unwrap_or_default();
        }

        if let Some(bins) = cargo.bins {
            //TODO: maybe we want the path too
            result.entry_points = bins.into_iter().map(|b| b.name).collect();
        }

        result
    }
}

fn parse_package_json(content: &str) -> Result<ParsedManifest, ParseError> {
    let pkg: PackageJson = serde_json::from_str(content)?;
    Ok(pkg.into())
}

impl From<PackageJson> for ParsedManifest {
    fn from(pkg: PackageJson) -> Self {
        let mut result = ParsedManifest {
            names: vec![pkg.name],
            version: pkg.version,
            workspace_members: Vec::new(),
            metadata: ManifestMetadata {
                description: pkg.description,
                authors: pkg.author.into_iter().collect(),
                license: pkg.license,
            },
            entry_points: Vec::new(),
            scripts: pkg.scripts.unwrap_or_default(),
            toolchain_version: pkg.engines.and_then(|e| e.node),
        };

        if let Some(ws) = pkg.workspaces {
            result.workspace_members = match ws {
                PackageJsonWorkspaces::Array(arr) => arr,
                PackageJsonWorkspaces::Object { packages } => packages,
            };
        }

        if let Some(bin) = pkg.bin {
            result.entry_points = match bin {
                PackageJsonBin::Single(s) => vec![s],
                PackageJsonBin::Multiple(map) => map.keys().cloned().collect(),
            };
        }

        result
    }
}

fn parse_pyproject_toml(content: &str) -> Result<ParsedManifest, ParseError> {
    let pyproject: PyprojectToml = toml::from_str(content)?;

    if let Some(project) = pyproject.project {
        Ok(project.into())
    } else {
        Ok(ParsedManifest::default())
    }
}

impl From<PythonProject> for ParsedManifest {
    fn from(project: PythonProject) -> Self {
        let authors = project
            .authors
            .unwrap_or_default()
            .into_iter()
            .map(|a| match a {
                PythonAuthor::Simple(s) => s,
                PythonAuthor::Detailed { name, .. } => name,
            })
            .collect();

        let license = project.license.and_then(|l| match l {
            PythonLicense::Simple(s) => Some(s),
            PythonLicense::Detailed { text, .. } => text,
        });

        let entry_points = project
            .scripts
            .as_ref()
            .map(|scripts| scripts.keys().cloned().collect())
            .unwrap_or_default();

        ParsedManifest {
            names: vec![project.name],
            version: project.version,
            workspace_members: Vec::new(),
            metadata: ManifestMetadata {
                description: project.description,
                authors,
                license,
            },
            entry_points,
            scripts: project.scripts.unwrap_or_default(),
            toolchain_version: project.requires_python,
        }
    }
}

fn parse_go_mod(content: &str) -> Result<ParsedManifest, ParseError> {
    let mut names = Vec::new();
    let mut toolchain_version = None;

    for line in content.lines() {
        let trimmed = line.trim();

        if let Some(module) = trimmed.strip_prefix("module ") {
            names.push(module.trim().to_string());
        } else if let Some(version) = trimmed.strip_prefix("go ") {
            toolchain_version = Some(version.trim().to_string());
        }
    }

    Ok(ParsedManifest {
        names,
        version: None,
        workspace_members: Vec::new(),
        metadata: ManifestMetadata::default(),
        entry_points: Vec::new(),
        scripts: HashMap::new(),
        toolchain_version,
    })
}
