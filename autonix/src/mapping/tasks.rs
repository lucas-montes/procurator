use std::path::Path;

use crate::mapping::{ParseError, Parseable};

/// Build system files that describe how to build a project
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum TaskFile {
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

/// Build system types that can be auto-detected
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildSystem {
    Make,
    CMake,
    Meson,
    Bazel,
    Just,
    Zig,
    Rake,
    Ant,
    Task,
}

/// Parsed build file information
#[derive(Debug, Clone, Default)]
pub struct ParsedTaskFile {
    /// Type of build system
    pub build_system: Option<BuildSystem>,

    /// Build targets found (e.g., "all", "test", "clean")
    pub targets: Vec<String>,

    /// System dependencies mentioned (e.g., "pkg-config", "openssl")
    pub system_deps: Vec<String>,
}

//TODO: use the extractors from autonix <https://github.com/davidabram/autonix/blob/main/src/detection/task_runner.rs>
impl Parseable for TaskFile {
    type Output = ParsedTaskFile;

    fn parse(&self, path: &Path) -> Result<Self::Output, ParseError> {
        let content = std::fs::read_to_string(path)?;

        let build_system = match self {
            Self::Makefile | Self::GNUmakefile => Some(BuildSystem::Make),
            Self::CMakeLists => Some(BuildSystem::CMake),
            Self::MesonBuild => Some(BuildSystem::Meson),
            Self::BazelBuild | Self::BazelWorkspace => Some(BuildSystem::Bazel),
            Self::Justfile => Some(BuildSystem::Just),
            Self::BuildZig => Some(BuildSystem::Zig),
            Self::Rakefile => Some(BuildSystem::Rake),
            Self::AntBuildXml => Some(BuildSystem::Ant),
            Self::Taskfile => Some(BuildSystem::Task),
        };

        let mut result = ParsedTaskFile {
            build_system,
            targets: Vec::new(),
            system_deps: Vec::new(),
        };

        // Extract targets and dependencies based on build system
        match self {
            Self::Makefile | Self::GNUmakefile => {
                parse_makefile(&content, &mut result);
            }
            Self::CMakeLists => {
                parse_cmake(&content, &mut result);
            }
            _ => {
                // For other build systems, just return the type
            }
        }

        Ok(result)
    }
}

/// Parse Makefile to extract targets and system deps
fn parse_makefile(content: &str, result: &mut ParsedTaskFile) {
    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments and empty lines
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Extract targets (lines ending with ':')
        if let Some(target_part) = trimmed.split(':').next() {
            if !target_part.contains('=') && !target_part.contains('$') {
                let target = target_part.trim();
                if !target.is_empty() && !target.starts_with('.') {
                    result.targets.push(target.to_string());
                }
            }
        }

        // Extract common system dependencies from variable assignments or commands
        for dep in ["pkg-config", "openssl", "curl", "postgresql", "sqlite", "zlib"] {
            if trimmed.to_lowercase().contains(dep) {
                if !result.system_deps.contains(&dep.to_string()) {
                    result.system_deps.push(dep.to_string());
                }
            }
        }
    }
}

/// Parse CMakeLists.txt to extract find_package calls
fn parse_cmake(content: &str, result: &mut ParsedTaskFile) {
    for line in content.lines() {
        let trimmed = line.trim();

        // Extract find_package() calls
        if let Some(pkg_start) = trimmed.find("find_package(") {
            let rest = &trimmed[pkg_start + 13..];
            if let Some(pkg_end) = rest.find(')') {
                let pkg_content = &rest[..pkg_end];
                // Take first word (package name), handle REQUIRED/COMPONENTS
                if let Some(pkg_name) = pkg_content.split_whitespace().next() {
                    let pkg_lower = pkg_name.to_lowercase();
                    if !result.system_deps.contains(&pkg_lower) {
                        result.system_deps.push(pkg_lower);
                    }
                }
            }
        }
    }
}

impl TryFrom<&str> for TaskFile {
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
