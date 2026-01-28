use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

use crate::mapping::{Language, PackageManager, ParseError, Parseable};

/// Manifest files declare dependencies and project metadata
/// These are the primary files that tell us what a project needs
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
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
    pub manifest_type: ManifestFile,
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
    scripts: Option<HashMap<String, String>>,
    workspaces: Option<PackageJsonWorkspaces>,
    engines: Option<PackageJsonEngines>,
    bin: Option<PackageJsonBin>,
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
            Self::CargoToml => parse_cargo_toml(&content, self.clone()),
            Self::PackageJson => parse_package_json(&content, self.clone()),
            Self::PyprojectToml => parse_pyproject_toml(&content, self.clone()),
            Self::GoMod => parse_go_mod(&content, self.clone()),

            // Version files (simple single-line reads)
            Self::PythonVersion | Self::RuntimeTxt => Ok(ParsedManifest {
                manifest_type: self.clone(),
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
            | Self::BuildSbt => {
                let mut result = ParsedManifest::default();
                result.manifest_type = self.clone();
                Ok(result)
            }
        }
    }
}

impl Default for ParsedManifest {
    fn default() -> Self {
        Self {
            manifest_type: ManifestFile::CargoToml, // Default fallback
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

fn parse_cargo_toml(content: &str, manifest_type: ManifestFile) -> Result<ParsedManifest, ParseError> {
    let cargo: CargoToml = toml::from_str(content)?;
    let mut result = ParsedManifest::from(cargo);
    result.manifest_type = manifest_type;
    Ok(result)
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

fn parse_package_json(content: &str, manifest_type: ManifestFile) -> Result<ParsedManifest, ParseError> {
    let pkg: PackageJson = serde_json::from_str(content)?;
    let mut result = ParsedManifest::from(pkg);
    result.manifest_type = manifest_type;
    Ok(result)
}

impl From<PackageJson> for ParsedManifest {
    fn from(pkg: PackageJson) -> Self {
        let mut result = ParsedManifest {
            manifest_type: ManifestFile::PackageJson,
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

fn parse_pyproject_toml(content: &str, manifest_type: ManifestFile) -> Result<ParsedManifest, ParseError> {
    let pyproject: PyprojectToml = toml::from_str(content)?;

    if let Some(project) = pyproject.project {
        let mut result = ParsedManifest::from(project);
        result.manifest_type = manifest_type;
        Ok(result)
    } else {
        let mut result = ParsedManifest::default();
        result.manifest_type = manifest_type;
        Ok(result)
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
            manifest_type: ManifestFile::PyprojectToml,
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

fn parse_go_mod(content: &str, manifest_type: ManifestFile) -> Result<ParsedManifest, ParseError> {
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
        manifest_type,
        names,
        version: None,
        workspace_members: Vec::new(),
        metadata: ManifestMetadata::default(),
        entry_points: Vec::new(),
        scripts: HashMap::new(),
        toolchain_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("manifests")
    }

    #[test]
    fn test_parse_cargo_toml() {
        let path = fixtures_path().join("Cargo.toml");
        let manifest = ManifestFile::CargoToml;
        let result = manifest.parse(&path).expect("Failed to parse Cargo.toml");

        assert_eq!(result.names, vec!["test-rust-project"]);
        assert_eq!(result.version, Some("0.1.0".to_string()));
        assert_eq!(result.toolchain_version, Some("2021".to_string()));
        assert_eq!(
            result.metadata.description,
            Some("A test Rust project".to_string())
        );
        assert_eq!(result.metadata.authors.len(), 2);
        assert!(result.metadata.authors.contains(&"Alice <alice@example.com>".to_string()));
        assert!(result.metadata.authors.contains(&"Bob <bob@example.com>".to_string()));
        assert_eq!(result.metadata.license, Some("MIT".to_string()));
        assert_eq!(result.entry_points.len(), 2);
        assert!(result.entry_points.contains(&"test-binary".to_string()));
        assert!(result.entry_points.contains(&"another-binary".to_string()));
        assert!(result.workspace_members.is_empty());
    }

    #[test]
    fn test_parse_cargo_workspace() {
        let path = fixtures_path().join("Cargo-workspace.toml");
        let manifest = ManifestFile::CargoToml;
        let result = manifest.parse(&path).expect("Failed to parse Cargo workspace");

        assert!(result.names.is_empty());
        assert!(result.version.is_none());
        assert_eq!(result.workspace_members.len(), 2);
        assert!(result.workspace_members.contains(&"crates/*".to_string()));
        assert!(result.workspace_members.contains(&"tools/cli".to_string()));
    }

    #[test]
    fn test_parse_package_json() {
        let path = fixtures_path().join("package.json");
        let manifest = ManifestFile::PackageJson;
        let result = manifest.parse(&path).expect("Failed to parse package.json");

        assert_eq!(result.names, vec!["test-js-project"]);
        assert_eq!(result.version, Some("2.1.0".to_string()));
        assert_eq!(
            result.metadata.description,
            Some("A test JavaScript project".to_string())
        );
        assert_eq!(result.metadata.authors, vec!["Charlie <charlie@example.com>"]);
        assert_eq!(result.metadata.license, Some("Apache-2.0".to_string()));
        assert_eq!(result.toolchain_version, Some(">=18.0.0".to_string()));

        // Check scripts
        assert_eq!(result.scripts.len(), 4);
        assert_eq!(result.scripts.get("test"), Some(&"jest".to_string()));
        assert_eq!(result.scripts.get("lint"), Some(&"eslint .".to_string()));
        assert_eq!(result.scripts.get("build"), Some(&"webpack".to_string()));
        assert_eq!(result.scripts.get("start"), Some(&"node index.js".to_string()));

        // Check entry points (bin)
        assert_eq!(result.entry_points.len(), 2);
        assert!(result.entry_points.contains(&"test-cli".to_string()));
        assert!(result.entry_points.contains(&"another-tool".to_string()));

        assert!(result.workspace_members.is_empty());
    }

    #[test]
    fn test_parse_package_json_workspace_array() {
        let path = fixtures_path().join("package-workspace.json");
        let manifest = ManifestFile::PackageJson;
        let result = manifest.parse(&path).expect("Failed to parse package workspace");

        assert_eq!(result.names, vec!["test-monorepo"]);
        assert_eq!(result.workspace_members.len(), 2);
        assert!(result.workspace_members.contains(&"packages/*".to_string()));
        assert!(result.workspace_members.contains(&"apps/*".to_string()));
        assert_eq!(result.scripts.get("test"), Some(&"npm run test --workspaces".to_string()));
    }

    #[test]
    fn test_parse_package_json_workspace_object() {
        let path = fixtures_path().join("package-workspace-object.json");
        let manifest = ManifestFile::PackageJson;
        let result = manifest.parse(&path).expect("Failed to parse package workspace object");

        assert_eq!(result.names, vec!["test-monorepo-obj"]);
        assert_eq!(result.workspace_members.len(), 2);
        assert!(result.workspace_members.contains(&"libs/*".to_string()));
        assert!(result.workspace_members.contains(&"services/*".to_string()));
    }

    #[test]
    fn test_parse_pyproject_toml() {
        let path = fixtures_path().join("pyproject.toml");
        let manifest = ManifestFile::PyprojectToml;
        let result = manifest.parse(&path).expect("Failed to parse pyproject.toml");

        assert_eq!(result.names, vec!["test-python-project"]);
        assert_eq!(result.version, Some("3.2.1".to_string()));
        assert_eq!(
            result.metadata.description,
            Some("A test Python project".to_string())
        );
        assert_eq!(result.metadata.authors.len(), 2);
        assert!(result.metadata.authors.contains(&"David".to_string()));
        assert!(result.metadata.authors.contains(&"Eve".to_string()));
        assert_eq!(result.metadata.license, Some("MIT".to_string()));
        assert_eq!(result.toolchain_version, Some(">=3.9".to_string()));

        // Check entry points (scripts)
        assert_eq!(result.entry_points.len(), 2);
        assert!(result.entry_points.contains(&"test-cli".to_string()));
        assert!(result.entry_points.contains(&"another-tool".to_string()));

        assert_eq!(result.scripts.len(), 2);
        assert_eq!(
            result.scripts.get("test-cli"),
            Some(&"test_package.main:cli".to_string())
        );
        assert_eq!(
            result.scripts.get("another-tool"),
            Some(&"test_package.tools:main".to_string())
        );
    }

    #[test]
    fn test_parse_pyproject_toml_simple() {
        let path = fixtures_path().join("pyproject-simple.toml");
        let manifest = ManifestFile::PyprojectToml;
        let result = manifest.parse(&path).expect("Failed to parse simple pyproject.toml");

        assert_eq!(result.names, vec!["simple-python-app"]);
        assert_eq!(result.version, Some("1.0.0".to_string()));
        assert_eq!(result.metadata.authors, vec!["Frank"]);
        assert_eq!(result.metadata.license, Some("BSD-3-Clause".to_string()));
        assert_eq!(result.toolchain_version, Some(">=3.11".to_string()));
        assert!(result.entry_points.is_empty());
        assert!(result.scripts.is_empty());
    }

    #[test]
    fn test_parse_go_mod() {
        let path = fixtures_path().join("go.mod");
        let manifest = ManifestFile::GoMod;
        let result = manifest.parse(&path).expect("Failed to parse go.mod");

        assert_eq!(result.names, vec!["github.com/example/test-go-project"]);
        assert!(result.version.is_none());
        assert_eq!(result.toolchain_version, Some("1.21".to_string()));
        assert!(result.workspace_members.is_empty());
        assert!(result.entry_points.is_empty());
        assert!(result.scripts.is_empty());
    }

    #[test]
    fn test_parse_python_version() {
        let path = fixtures_path().join(".python-version");
        let manifest = ManifestFile::PythonVersion;
        let result = manifest.parse(&path).expect("Failed to parse .python-version");

        assert!(result.names.is_empty());
        assert!(result.version.is_none());
        assert_eq!(result.toolchain_version, Some("3.11.5".to_string()));
        assert!(result.workspace_members.is_empty());
        assert!(result.entry_points.is_empty());
        assert!(result.scripts.is_empty());
    }

    #[test]
    fn test_parse_runtime_txt() {
        let path = fixtures_path().join("runtime.txt");
        let manifest = ManifestFile::RuntimeTxt;
        let result = manifest.parse(&path).expect("Failed to parse runtime.txt");

        assert!(result.names.is_empty());
        assert!(result.version.is_none());
        assert_eq!(result.toolchain_version, Some("python-3.10.12".to_string()));
        assert!(result.workspace_members.is_empty());
        assert!(result.entry_points.is_empty());
        assert!(result.scripts.is_empty());
    }

    #[test]
    fn test_parse_package_json_single_bin() {
        let path = fixtures_path().join("package-single-bin.json");
        let manifest = ManifestFile::PackageJson;
        let result = manifest.parse(&path).expect("Failed to parse package.json with single bin");

        assert_eq!(result.names, vec!["simple-cli"]);
        assert_eq!(result.entry_points.len(), 1);
        assert_eq!(result.entry_points[0], "./cli.js");
    }

    #[test]
    fn test_parse_cargo_minimal() {
        let path = fixtures_path().join("Cargo-minimal.toml");
        let manifest = ManifestFile::CargoToml;
        let result = manifest.parse(&path).expect("Failed to parse minimal Cargo.toml");

        assert_eq!(result.names, vec!["minimal-crate"]);
        assert_eq!(result.version, Some("0.1.0".to_string()));
        assert_eq!(result.toolchain_version, Some("2021".to_string()));
        assert!(result.metadata.description.is_none());
        assert!(result.metadata.authors.is_empty());
        assert!(result.metadata.license.is_none());
        assert!(result.entry_points.is_empty());
    }

}

/// Convert ManifestFile to Language
impl From<&ManifestFile> for Language {
    fn from(manifest: &ManifestFile) -> Self {
        match manifest {
            ManifestFile::CargoToml => Language::Rust,
            ManifestFile::PackageJson => Language::JavaScript,
            ManifestFile::PyprojectToml
            | ManifestFile::SetupPy
            | ManifestFile::SetupCfg
            | ManifestFile::RequirementsTxt
            | ManifestFile::Pipfile
            | ManifestFile::PythonVersion
            | ManifestFile::RuntimeTxt
            | ManifestFile::CondaYaml
            | ManifestFile::EnvironmentYml => Language::Python,
            ManifestFile::GoMod => Language::Go,
            ManifestFile::PomXml | ManifestFile::BuildGradle | ManifestFile::BuildGradleKts | ManifestFile::BuildSbt => Language::Java,
            ManifestFile::Csproj | ManifestFile::Fsproj | ManifestFile::Sln => Language::CSharp,
            ManifestFile::Gemfile => Language::Ruby,
            ManifestFile::ComposerJson => Language::PHP,
        }
    }
}

/// Convert ManifestFile to PackageManager
impl From<&ManifestFile> for PackageManager {
    fn from(manifest: &ManifestFile) -> Self {
        match manifest {
            ManifestFile::CargoToml => PackageManager::Cargo,
            ManifestFile::PackageJson => PackageManager::Npm, // Default, can be overridden by lockfile detection
            ManifestFile::PyprojectToml => PackageManager::Poetry, // Modern Python, likely Poetry or similar
            ManifestFile::SetupPy | ManifestFile::SetupCfg => PackageManager::Pip,
            ManifestFile::RequirementsTxt => PackageManager::Pip,
            ManifestFile::Pipfile => PackageManager::Pipenv,
            ManifestFile::PythonVersion | ManifestFile::RuntimeTxt => PackageManager::Pip, // Version files, default to pip
            ManifestFile::CondaYaml | ManifestFile::EnvironmentYml => PackageManager::Conda,
            ManifestFile::GoMod => PackageManager::GoModules,
            ManifestFile::PomXml => PackageManager::Maven,
            ManifestFile::BuildGradle | ManifestFile::BuildGradleKts => PackageManager::Gradle,
            ManifestFile::BuildSbt => PackageManager::Sbt,
            ManifestFile::Csproj | ManifestFile::Fsproj | ManifestFile::Sln => PackageManager::Nuget,
            ManifestFile::Gemfile => PackageManager::Bundler,
            ManifestFile::ComposerJson => PackageManager::Composer,
        }
    }
}
