//! Control-plane startup configuration with Nix-declared worker inventory.
//!
//! The JSON file is rendered by `nix/modules/control-plane.nix`. The `workers`
//! array carries the deterministic worker inventory (see
//! `crate::registry::DeclaredWorker`); it is validated fail-fast at startup,
//! mirroring `worker/src/config.rs`. Unknown keys (e.g. the `role` envelope)
//! are ignored so the file stays forward-compatible in both directions.
//!
//! Seeding the parsed inventory into `Node`'s registry happens in T03; this
//! module only parses and validates.

use std::{
    fmt::{Display, Formatter},
    net::SocketAddr,
    path::Path,
};

use serde::Deserialize;

use super::registry::{DeclaredWorker, RegistryError, WorkerId};

/// One Nix-declared worker.
///
/// Serialized as `{ "id": "...", "addr": "...", "labels": [...] }`; `addr`
/// and `labels` are optional. `addr` is informational only in pull mode (see
/// `DeclaredWorker`), kept for logs, status views, and a future push
/// transport.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkerEntry {
    /// Worker id, matching the Nix `cluster.vms` name.
    pub id: String,
    /// Informational deployment address hint from Nix, if any.
    #[serde(default)]
    pub addr: Option<String>,
    /// Scheduling labels from Nix (reserved for a later scheduler plan).
    #[serde(default)]
    pub labels: Vec<String>,
}

impl WorkerEntry {
    /// Converts into the T01 registry inventory type.
    ///
    /// # Errors
    ///
    /// Returns `RegistryError::EmptyId` when `id` is empty or whitespace-only.
    pub fn into_declared(self) -> Result<DeclaredWorker, RegistryError> {
        let mut declared = DeclaredWorker::new(WorkerId::new(self.id)?);
        if let Some(addr) = self.addr {
            declared = declared.with_addr_hint(addr);
        }
        if !self.labels.is_empty() {
            declared = declared.with_labels(self.labels);
        }
        Ok(declared)
    }
}

/// Control-plane startup configuration.
#[derive(Debug, Deserialize)]
pub struct Config {
    /// Human-readable name for this control-plane node.
    pub hostname: String,
    /// Address the Cap'n Proto `Master` listener binds to.
    pub addr: SocketAddr,
    /// Deterministic worker inventory; empty when unconfigured.
    #[serde(default)]
    pub workers: Vec<WorkerEntry>,
}

impl Config {
    /// Loads, parses, and validates the JSON config file, exiting the
    /// process with a logged error on any failure (fail-fast startup).
    pub fn from_file(path: impl AsRef<Path> + std::fmt::Debug) -> Self {
        let contents = std::fs::read(&path).unwrap_or_else(|e| {
            tracing::error!(path = ?path, error = %e, "Could not read config");
            std::process::exit(1);
        });

        let config: Self = serde_json::from_slice(&contents).unwrap_or_else(|e| {
            tracing::error!(path = ?path, error = %e, "Failed to parse config");
            std::process::exit(1);
        });

        config.validate().unwrap_or_else(|e| {
            tracing::error!(path = ?path, error = %e, "Invalid control plane config");
            std::process::exit(1);
        });

        config
    }

    /// Converts the configured inventory into registry declarations.
    ///
    /// # Errors
    ///
    /// Returns `RegistryError::EmptyId` for blank ids and
    /// `RegistryError::Duplicate` for repeated ids.
    pub fn declared_workers(&self) -> Result<Vec<DeclaredWorker>, RegistryError> {
        let mut declared = Vec::with_capacity(self.workers.len());
        for entry in &self.workers {
            let worker = entry.clone().into_declared()?;
            if declared
                .iter()
                .any(|w: &DeclaredWorker| w.id() == worker.id())
            {
                return Err(RegistryError::Duplicate(worker.id().clone()));
            }
            declared.push(worker);
        }
        Ok(declared)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.hostname.trim().is_empty() {
            return Err(ConfigError::EmptyHostname);
        }
        let mut seen: Vec<&str> = Vec::with_capacity(self.workers.len());
        for entry in &self.workers {
            if entry.id.trim().is_empty() {
                return Err(ConfigError::EmptyWorkerId);
            }
            if seen.contains(&entry.id.as_str()) {
                return Err(ConfigError::DuplicateWorkerId(entry.id.clone()));
            }
            seen.push(&entry.id);
        }
        Ok(())
    }
}

/// Startup validation failures with explicit operator-facing messages.
#[derive(Debug, PartialEq, Eq)]
enum ConfigError {
    /// `hostname` was empty or whitespace-only.
    EmptyHostname,
    /// A worker entry id was empty or whitespace-only.
    EmptyWorkerId,
    /// A worker id was declared more than once.
    DuplicateWorkerId(String),
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyHostname => f.write_str("hostname must not be empty"),
            Self::EmptyWorkerId => f.write_str("worker id must not be empty"),
            Self::DuplicateWorkerId(id) => {
                write!(f, "worker id is declared more than once: {id}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::Config;

    fn config_json(workers_fragment: &str) -> String {
        format!(
            r#"{{
                "hostname": "cp-1",
                "addr": "127.0.0.1:5000",
                "role": {{ "Master": {{ "peers_addr": [] }} }},
                "workers": {workers_fragment}
            }}"#,
        )
    }

    fn parse(workers_fragment: &str) -> Config {
        let config: Config =
            serde_json::from_str(&config_json(workers_fragment)).expect("test config must parse");
        config.validate().expect("test config must validate");
        config
    }

    #[test]
    fn parses_workers_with_optional_fields() {
        let config = parse(
            r#"[
                { "id": "w1", "addr": "192.168.1.11:8080", "labels": ["gpu"] },
                { "id": "w2" }
            ]"#,
        );

        assert_eq!(config.hostname, "cp-1");
        assert_eq!(config.addr.to_string(), "127.0.0.1:5000");

        let declared = config.declared_workers().expect("inventory must convert");
        assert_eq!(declared.len(), 2);
        assert_eq!(declared[0].id().as_str(), "w1");
        assert_eq!(declared[0].addr_hint(), Some("192.168.1.11:8080"));
        assert_eq!(declared[0].labels(), &["gpu".to_string()]);
        assert_eq!(declared[1].id().as_str(), "w2");
        assert_eq!(declared[1].addr_hint(), None);
    }

    #[test]
    fn tolerates_missing_workers_key() {
        let config: Config =
            serde_json::from_str(r#"{"hostname": "cp-1", "addr": "127.0.0.1:5000"}"#)
                .expect("config without workers must parse");
        assert!(config.workers.is_empty());
        assert!(
            config
                .declared_workers()
                .expect("empty inventory")
                .is_empty()
        );
    }

    #[test]
    fn rejects_duplicate_worker_ids() {
        let config: Config =
            serde_json::from_str(&config_json(r#"[{ "id": "w1" }, { "id": "w1" }]"#))
                .expect("duplicate config still parses as JSON");

        let err = format!("{}", config.validate().expect_err("duplicates must fail"));
        assert!(err.contains("w1"), "unexpected error: {err}");

        let err = config
            .declared_workers()
            .expect_err("duplicates must fail conversion");
        assert_eq!(format!("{err}"), "worker id is already declared: w1",);
    }

    #[test]
    fn rejects_empty_worker_ids() {
        for id in [String::new(), "   ".to_string()] {
            let fragment = format!(r#"[{{ "id": {id:?} }}]"#);
            let config: Config = serde_json::from_str(&config_json(&fragment))
                .expect("blank-id config still parses as JSON");
            assert!(
                config.validate().is_err(),
                "blank id must fail validation: {id:?}"
            );
            assert!(
                config.declared_workers().is_err(),
                "blank id must fail conversion: {id:?}"
            );
        }
    }

    #[test]
    fn rejects_empty_hostname() {
        let config: Config =
            serde_json::from_str(r#"{"hostname": "  ", "addr": "127.0.0.1:5000", "workers": []}"#)
                .expect("blank-hostname config still parses as JSON");
        let err = format!(
            "{}",
            config.validate().expect_err("blank hostname must fail")
        );
        assert!(err.contains("hostname"), "unexpected error: {err}");
    }
}
