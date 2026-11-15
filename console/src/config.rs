//! Console configuration (JSON, rendered by `nix/modules/console.nix` or
//! written by hand for manual runs).
//!
//! ```json
//! {
//!   "http_addr": "127.0.0.1:8095",
//!   "bootstrap_entries": 2000,
//!   "stale_after_millis": 15000,
//!   "sources": [
//!     { "name": "cp", "service": "control_plane", "kind": { "file": { "path": "/tmp/pcr/cp.log" } } },
//!     { "name": "w1", "service": "worker:worker-1",
//!       "kind": { "journal_gateway": { "url": "http://10.0.0.11:19531", "unit": "procurator-worker.service" } } },
//!     { "name": "w2", "service": "worker:worker-2", "kind": { "journal_local": { "unit": "procurator-worker.service" } } }
//!   ]
//! }
//! ```

use std::{
    fmt::{Display, Formatter},
    net::SocketAddr,
    path::{Path, PathBuf},
};

use serde::Deserialize;

/// Where one service's logs are read from.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A JSON log file written by `tracing-appender` (manual runs). With
    /// daily rotation the newest `<path>.<date>` file is followed.
    File { path: PathBuf },
    /// `systemd-journal-gatewayd` of the host running the service.
    JournalGateway { url: String, unit: String },
    /// `journalctl -f` on the console host.
    JournalLocal { unit: String },
}

impl Display for SourceKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File { path } => write!(f, "file {}", path.display()),
            Self::JournalGateway { url, unit } => write!(f, "journal-gatewayd {url} ({unit})"),
            Self::JournalLocal { unit } => write!(f, "journalctl ({unit})"),
        }
    }
}

/// One configured log source.
#[derive(Debug, Deserialize)]
pub struct SourceConfig {
    name: String,
    /// `control_plane` or `worker:<id>`: who wrote the lines, used to
    /// resolve the `self` endpoint.
    service: String,
    kind: SourceKind,
}

impl SourceConfig {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The endpoint this source's `self` stands for: `cp` or `worker:<id>`.
    pub fn endpoint(&self) -> &str {
        if self.service == "control_plane" {
            "cp"
        } else {
            &self.service
        }
    }

    pub fn kind(&self) -> &SourceKind {
        &self.kind
    }

    pub fn into_kind(self) -> SourceKind {
        self.kind
    }
}

fn default_http_addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 8095))
}

fn default_bootstrap_entries() -> u64 {
    2000
}

fn default_stale_after_millis() -> u64 {
    15_000
}

/// Console startup configuration.
#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default = "default_http_addr")]
    http_addr: SocketAddr,
    /// How many recent lines each source reads back at startup to rebuild
    /// the current state (state lines repeat every few seconds, so a few
    /// thousand lines is plenty).
    #[serde(default = "default_bootstrap_entries")]
    bootstrap_entries: u64,
    /// A worker with no state line for this long is shown as unhealthy.
    #[serde(default = "default_stale_after_millis")]
    stale_after_millis: u64,
    sources: Vec<SourceConfig>,
}

impl Config {
    /// Reads and validates the config file.
    ///
    /// # Errors
    ///
    /// Returns a readable message when the file can't be read or parsed, or
    /// when a source is invalid.
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let bytes =
            std::fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
        let config: Self = serde_json::from_slice(&bytes)
            .map_err(|err| format!("cannot parse {}: {err}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if self.sources.is_empty() {
            return Err("at least one source is required".to_owned());
        }
        for (index, source) in self.sources.iter().enumerate() {
            if source.name.trim().is_empty() {
                return Err(format!("source #{index} has an empty name"));
            }
            if self.sources[..index]
                .iter()
                .any(|other| other.name == source.name)
            {
                return Err(format!("source name {:?} is used twice", source.name));
            }
            let valid_service = source.service == "control_plane"
                || source
                    .service
                    .strip_prefix("worker:")
                    .is_some_and(|id| !id.trim().is_empty());
            if !valid_service {
                return Err(format!(
                    "source {:?}: service must be \"control_plane\" or \"worker:<id>\", got {:?}",
                    source.name, source.service
                ));
            }
        }
        Ok(())
    }

    pub fn http_addr(&self) -> SocketAddr {
        self.http_addr
    }

    pub fn bootstrap_entries(&self) -> u64 {
        self.bootstrap_entries
    }

    pub fn stale_after_millis(&self) -> u64 {
        self.stale_after_millis
    }

    pub fn sources(&self) -> &[SourceConfig] {
        &self.sources
    }

    pub fn into_sources(self) -> Vec<SourceConfig> {
        self.sources
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, SourceKind};

    fn parse(json: &str) -> Result<Config, String> {
        let config: Config = serde_json::from_str(json).map_err(|err| err.to_string())?;
        config.validate()?;
        Ok(config)
    }

    #[test]
    fn parses_every_source_kind() {
        let config = parse(
            r#"{"sources": [
                {"name": "cp", "service": "control_plane", "kind": {"file": {"path": "/tmp/cp.log"}}},
                {"name": "w1", "service": "worker:w1", "kind": {"journal_gateway": {"url": "http://h:19531", "unit": "w.service"}}},
                {"name": "w2", "service": "worker:w2", "kind": {"journal_local": {"unit": "w.service"}}}
            ]}"#,
        )
        .expect("valid config");
        assert_eq!(config.http_addr().to_string(), "127.0.0.1:8095");
        assert_eq!(config.sources()[0].endpoint(), "cp");
        assert_eq!(config.sources()[1].endpoint(), "worker:w1");
        assert!(matches!(
            config.sources()[2].kind(),
            SourceKind::JournalLocal { .. }
        ));
    }

    #[test]
    fn rejects_duplicate_names_and_bad_services() {
        let duplicate = parse(
            r#"{"sources": [
                {"name": "a", "service": "control_plane", "kind": {"file": {"path": "x"}}},
                {"name": "a", "service": "worker:w1", "kind": {"file": {"path": "y"}}}
            ]}"#,
        );
        assert!(duplicate.is_err());
        let bad = parse(
            r#"{"sources": [{"name": "a", "service": "worker:", "kind": {"file": {"path": "x"}}}]}"#,
        );
        assert!(bad.is_err());
        assert!(parse(r#"{"sources": []}"#).is_err());
    }
}
