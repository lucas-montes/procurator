//! The monolith's config: where each service's own config file is.
//! Why: each service keeps the one file its standalone binary and NixOS
//! module already use; the monolith only says which ones.

use std::fmt;
use std::path::{Path, PathBuf};

/// Paths to the three services' TOML files, and optionally the control
/// plane's JSON one.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    ci: PathBuf,
    repohub: PathBuf,
    cache: PathBuf,
    /// Why: optional; without it, deploys are sent to `deploy.url`.
    #[serde(default)]
    control_plane: Option<PathBuf>,
}

/// Why the monolith's config could not be read.
#[derive(Debug)]
pub struct ConfigError(String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "config: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Read `path`.
    ///
    /// # Errors
    ///
    /// When it is unreadable, not TOML, or lacks a key.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
        toml::from_str(&raw).map_err(|e| ConfigError(format!("{}: {e}", path.display())))
    }

    /// The CI service's config file.
    pub fn ci(&self) -> &Path {
        &self.ci
    }

    /// Repohub's config file.
    pub fn repohub(&self) -> &Path {
        &self.repohub
    }

    /// The cache's config file.
    pub fn cache(&self) -> &Path {
        &self.cache
    }

    /// The control plane's config file, when the monolith runs it.
    pub fn control_plane(&self) -> Option<&Path> {
        self.control_plane.as_deref()
    }
}
