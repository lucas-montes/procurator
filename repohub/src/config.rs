//! Runtime configuration loaded from a TOML file.
//! Why: one file is the single source of truth for local runs and NixOS.

use std::path::Path;

/// All knobs for the repohub service. See `nix/modules/repohub/service.nix`
/// for NixOS wiring. Built from [`FileConfig`] so IO and validation stay out
/// of deserialization.
#[derive(Debug, Clone)]
pub struct Config {
    /// Database connection URL (e.g. `sqlite:///var/lib/procurator-repohub/repohub.db`).
    pub database_url: String,
    /// Bind address, e.g. `0.0.0.0:8083`.
    pub bind_address: String,
    /// Domain used to render SSH/repo URLs (see `services/repository_service.rs`).
    pub domain: String,
    /// Where bare git repositories are served from (git-server root).
    pub repos_base_path: String,
}

/// Concrete config errors. Why: no `anyhow`, callers match on variants.
#[derive(Debug)]
pub enum ConfigError {
    /// Config file missing or unreadable.
    NotFound(String),
    /// TOML parse or value error (includes line info from the parser).
    Parse(String),
    /// A value failed startup validation.
    Invalid(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(m) => write!(f, "config file: {m}"),
            Self::Parse(m) => write!(f, "config parse: {m}"),
            Self::Invalid(m) => write!(f, "config invalid: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Raw file shape. Why: keeps IO/validation out of `Deserialize`.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    /// Database connection URL.
    #[serde(default = "default_database_url")]
    database_url: String,
    /// Bind address.
    #[serde(default = "default_bind_address")]
    bind_address: String,
    /// Domain used to render SSH/repo URLs.
    #[serde(default = "default_domain")]
    domain: String,
    /// Where bare git repositories are served from.
    #[serde(default = "default_repos_base_path")]
    repos_base_path: String,
}

impl Default for FileConfig {
    fn default() -> Self {
        Self {
            database_url: default_database_url(),
            bind_address: default_bind_address(),
            domain: default_domain(),
            repos_base_path: default_repos_base_path(),
        }
    }
}

impl Default for Config {
    /// Config with the documented defaults (matches the TOML keys).
    /// Why: keeps `Config::default()` working for dev runs.
    fn default() -> Self {
        let fc = FileConfig::default();
        Self {
            database_url: fc.database_url,
            bind_address: fc.bind_address,
            domain: fc.domain,
            repos_base_path: fc.repos_base_path,
        }
    }
}

impl Config {
    /// Load `--config <path>`. Why: file-only, no env fallback.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NotFound`] when the file is unreadable,
    /// [`ConfigError::Parse`] on TOML errors,
    /// [`ConfigError::Invalid`] on failed validation.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::NotFound(format!("{}: {e}", path.display())))?;
        let fc: FileConfig =
            toml::from_str(&raw).map_err(|e| ConfigError::Parse(format!("{e}")))?;
        Self::load(fc)
    }

    /// Resolve the raw file into a validated [`Config`].
    fn load(fc: FileConfig) -> Result<Self, ConfigError> {
        let cfg = Self {
            database_url: fc.database_url,
            bind_address: fc.bind_address,
            domain: fc.domain,
            repos_base_path: fc.repos_base_path,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    /// Fail-fast on values the server can't work with.
    fn validate(&self) -> Result<(), ConfigError> {
        self.bind_address
            .parse::<std::net::SocketAddr>()
            .map_err(|e| ConfigError::Invalid(format!("bind_address: {e}")))?;
        if self.database_url.trim().is_empty() {
            return Err(ConfigError::Invalid("database_url is empty".to_string()));
        }
        if self.repos_base_path.trim().is_empty() {
            return Err(ConfigError::Invalid("repos_base_path is empty".to_string()));
        }
        Ok(())
    }
}

fn default_database_url() -> String {
    "sqlite:///var/lib/procurator-repohub/repohub.db".to_string()
}

fn default_bind_address() -> String {
    "0.0.0.0:8083".to_string()
}

fn default_domain() -> String {
    "homelab".to_string()
}

fn default_repos_base_path() -> String {
    "/var/lib/git-server".to_string()
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigError};
    use std::path::PathBuf;

    /// Write a temp file that won't collide across parallel tests.
    fn temp_path(name: &str, body: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("pcr-repohub-test-{name}-{nanos}.toml"));
        std::fs::write(&path, body).expect("write temp config");
        path
    }

    #[test]
    fn defaults_match_documented_values() {
        let cfg = Config::default();
        assert_eq!(cfg.bind_address, "0.0.0.0:8083");
        assert_eq!(
            cfg.database_url,
            "sqlite:///var/lib/procurator-repohub/repohub.db"
        );
        assert_eq!(cfg.repos_base_path, "/var/lib/git-server");
        assert_eq!(cfg.domain, "homelab");
    }

    #[test]
    fn parses_all_fields() {
        let path = temp_path(
            "full",
            r#"
database_url = "sqlite:///tmp/repohub.db"
bind_address = "127.0.0.1:9001"
domain = "lab"
repos_base_path = "/srv/git"
"#,
        );
        let cfg = Config::from_file(&path).expect("full config must parse");
        assert_eq!(cfg.bind_address, "127.0.0.1:9001");
        assert_eq!(cfg.database_url, "sqlite:///tmp/repohub.db");
        assert_eq!(cfg.domain, "lab");
        assert_eq!(cfg.repos_base_path, "/srv/git");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn minimal_file_uses_defaults() {
        let path = temp_path("minimal", "");
        let cfg = Config::from_file(&path).expect("empty config uses defaults");
        assert_eq!(cfg.bind_address, "0.0.0.0:8083");
        assert_eq!(cfg.domain, "homelab");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_unknown_fields() {
        let path = temp_path(
            "unknown",
            "bind_address = \"0.0.0.0:8083\"\nwebhook_url = \"x\"\n",
        );
        let err = Config::from_file(&path).unwrap_err().to_string();
        assert!(err.contains("unknown field"), "got: {err}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_bad_bind_address() {
        let path = temp_path("badaddr", "bind_address = \"nope\"\n");
        let err = Config::from_file(&path).unwrap_err().to_string();
        assert!(err.contains("bind_address"), "got: {err}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_file_errors() {
        let path = PathBuf::from("/nonexistent-repohub.toml");
        match Config::from_file(&path) {
            Err(ConfigError::NotFound(m)) => assert!(m.contains("nonexistent"), "got: {m}"),
            Err(other) => panic!("expected NotFound, got {other:?}"),
            Ok(_) => panic!("expected NotFound"),
        }
    }
}
