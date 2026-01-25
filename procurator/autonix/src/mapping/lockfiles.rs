use std::path::Path;

use crate::mapping::{ParseError, Parseable};

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

pub struct ParsedLockFile;

impl Parseable for LockFile {
    type Output = ParsedLockFile;

    fn parse(&self, _path: &Path) -> Result<Self::Output, ParseError> {
        Ok(ParsedLockFile)
    }
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
