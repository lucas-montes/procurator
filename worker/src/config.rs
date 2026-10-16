use std::{
    net::SocketAddr,
    num::NonZeroU64,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::vmm::Factory;

#[derive(Debug, Deserialize)]
pub struct Config<F: Factory> {
    pub rpc_listen_addr: SocketAddr,
    pub health_tick_millis: NonZeroU64,
    /// This worker's id, matching the control-plane declared inventory.
    /// Required: blank ids fail startup (T07 sources it from Nix).
    pub worker_id: String,
    /// Control-plane address polled for assignments and heartbeats.
    /// Required: already emitted by `nix/lib/worker.nix` as `master_addr`.
    pub master_addr: SocketAddr,
    /// Interval between assignment polls.
    #[serde(default = "default_poll_interval_millis")]
    pub poll_interval_millis: NonZeroU64,
    /// Interval between heartbeat pushes.
    #[serde(default = "default_heartbeat_interval_millis")]
    pub heartbeat_interval_millis: NonZeroU64,
    pub proxy: ProxyConfig,
    pub vmm: F::Config,
}

fn default_poll_interval_millis() -> NonZeroU64 {
    NonZeroU64::new(2000).unwrap_or(NonZeroU64::MIN)
}

fn default_heartbeat_interval_millis() -> NonZeroU64 {
    NonZeroU64::new(5000).unwrap_or(NonZeroU64::MIN)
}

impl<F: Factory> Config<F> {
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
            tracing::error!(path = ?path, error = %e, "Invalid worker config");
            std::process::exit(1);
        });

        config
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.worker_id.trim().is_empty() {
            return Err(ConfigError::EmptyWorkerId);
        }
        self.proxy.validate(self.rpc_listen_addr)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProxyConfig {
    pub public_listen_addr: SocketAddr,
    pub tls_cert_path: PathBuf,
    pub tls_key_path: PathBuf,
    pub base_domain: String,
    pub jwt_hs256_secret: String,
    pub upstream_request_timeout_millis: NonZeroU64,
}

impl ProxyConfig {
    fn validate(&self, rpc_listen_addr: SocketAddr) -> Result<(), ConfigError> {
        if self.public_listen_addr == rpc_listen_addr {
            return Err(ConfigError::ListenerConflict {
                rpc_listen_addr,
                proxy_listen_addr: self.public_listen_addr,
            });
        }

        if self.jwt_hs256_secret.trim().is_empty() {
            return Err(ConfigError::EmptyProxyJwtSecret);
        }

        if self.base_domain.trim().is_empty() {
            return Err(ConfigError::EmptyProxyBaseDomain);
        }

        Ok(())
    }
}

#[derive(Debug)]
enum ConfigError {
    ListenerConflict {
        rpc_listen_addr: SocketAddr,
        proxy_listen_addr: SocketAddr,
    },
    EmptyProxyJwtSecret,
    EmptyProxyBaseDomain,
    EmptyWorkerId,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::ListenerConflict {
                rpc_listen_addr,
                proxy_listen_addr,
            } => write!(
                f,
                "rpc and proxy listeners must be different (rpc: {rpc_listen_addr}, proxy: {proxy_listen_addr})"
            ),
            ConfigError::EmptyProxyJwtSecret => {
                write!(f, "proxy.jwt_hs256_secret must not be empty")
            }
            ConfigError::EmptyProxyBaseDomain => {
                write!(f, "proxy.base_domain must not be empty")
            }
            ConfigError::EmptyWorkerId => {
                write!(f, "worker_id must not be empty")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, path::PathBuf};

    use super::{Config, ProxyConfig};
    use crate::test_support::FakeFactory;

    fn worker_json(extra: &str) -> String {
        format!(
            r#"{{
                "rpc_listen_addr": "127.0.0.1:8080",
                "health_tick_millis": 1000,
                "proxy": {{
                    "public_listen_addr": "0.0.0.0:8443",
                    "tls_cert_path": "/tmp/cert.pem",
                    "tls_key_path": "/tmp/key.pem",
                    "base_domain": "vm.example.test",
                    "jwt_hs256_secret": "super-secret",
                    "upstream_request_timeout_millis": 30000
                }},
                "vmm": {{}},
                {extra}
            }}"#,
        )
    }

    fn parse(extra: &str) -> Config<FakeFactory> {
        let config: Config<FakeFactory> =
            serde_json::from_str(&worker_json(extra)).expect("test config must parse");
        config.validate().expect("test config must validate");
        config
    }

    fn proxy_config() -> ProxyConfig {
        ProxyConfig {
            public_listen_addr: "0.0.0.0:8443".parse().expect("valid socket addr"),
            tls_cert_path: PathBuf::from("/tmp/procurator2-worker-test-cert.pem"),
            tls_key_path: PathBuf::from("/tmp/procurator2-worker-test-key.pem"),
            base_domain: "vm.example.test".to_string(),
            jwt_hs256_secret: "super-secret".to_string(),
            upstream_request_timeout_millis: std::num::NonZeroU64::new(30_000).expect("non-zero"),
        }
    }

    #[test]
    fn rejects_same_listener_for_rpc_and_proxy() {
        let mut proxy = proxy_config();
        let addr: SocketAddr = "127.0.0.1:8443".parse().expect("valid socket addr");
        proxy.public_listen_addr = addr;

        let result = proxy.validate(addr);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_empty_jwt_secret() {
        let mut proxy = proxy_config();
        proxy.jwt_hs256_secret = "   ".to_string();

        let rpc_addr: SocketAddr = "127.0.0.1:8080".parse().expect("valid socket addr");
        let result = proxy.validate(rpc_addr);

        assert!(result.is_err());
    }

    #[test]
    fn rejects_empty_base_domain() {
        let mut proxy = proxy_config();
        proxy.base_domain = " ".to_string();

        let rpc_addr: SocketAddr = "127.0.0.1:8080".parse().expect("valid socket addr");
        let result = proxy.validate(rpc_addr);

        assert!(result.is_err());
    }

    #[test]
    fn parses_pull_fields() {
        let config = parse(
            r#""worker_id": "w1", "master_addr": "127.0.0.1:5000", "poll_interval_millis": 2500, "heartbeat_interval_millis": 7000"#,
        );

        assert_eq!(config.worker_id, "w1");
        assert_eq!(config.master_addr.to_string(), "127.0.0.1:5000");
        assert_eq!(config.poll_interval_millis.get(), 2500);
        assert_eq!(config.heartbeat_interval_millis.get(), 7000);
    }

    #[test]
    fn defaults_poll_intervals() {
        let config = parse(r#""worker_id": "w1", "master_addr": "127.0.0.1:5000""#);

        assert_eq!(config.poll_interval_millis.get(), 2000);
        assert_eq!(config.heartbeat_interval_millis.get(), 5000);
    }

    #[test]
    fn rejects_missing_worker_id() {
        let result = serde_json::from_str::<Config<FakeFactory>>(&worker_json(
            r#""master_addr": "127.0.0.1:5000""#,
        ));
        assert!(result.is_err(), "missing worker_id must fail to parse");
    }

    #[test]
    fn rejects_blank_worker_id() {
        let config: Config<FakeFactory> = serde_json::from_str(&worker_json(
            r#""worker_id": "  ", "master_addr": "127.0.0.1:5000""#,
        ))
        .expect("blank worker_id still parses as JSON");
        assert!(
            config.validate().is_err(),
            "blank worker_id must fail validation"
        );
    }

    #[test]
    fn rejects_missing_master_addr() {
        let result =
            serde_json::from_str::<Config<FakeFactory>>(&worker_json(r#""worker_id": "w1""#));
        assert!(result.is_err(), "missing master_addr must fail to parse");
    }

    #[test]
    fn parses_nix_rendered_shape_for_real_backend() {
        use crate::ch::Factory as ChFactory;

        // Mirrors nix/lib/worker.nix mkWorkerConfig output key-for-key
        // (worker_id, master_addr, both intervals, proxy, vmm); if nix adds
        // a key, add it here too.
        let rendered = r#"{
            "rpc_listen_addr": "0.0.0.0:8080",
            "master_addr": "192.168.1.10:5000",
            "health_tick_millis": 1000,
            "worker_id": "worker-1",
            "poll_interval_millis": 2000,
            "heartbeat_interval_millis": 5000,
            "proxy": {
                "public_listen_addr": "0.0.0.0:8443",
                "tls_cert_path": "/var/lib/procurator-worker/tls/server.crt",
                "tls_key_path": "/var/lib/procurator-worker/tls/server.key",
                "base_domain": "vm.example.test",
                "jwt_hs256_secret": "super-secret",
                "upstream_request_timeout_millis": 30000
            },
            "vmm": {
                "binary_path": "/bin/cloud-hypervisor",
                "runtime_dir": "/run/procurator-worker",
                "state_dir": "/var/lib/procurator-worker",
                "bridge_name": "br0",
                "bridge_gateway": "10.0.0.1",
                "ip_pool_start": "10.0.0.2",
                "ip_pool_end": "10.255.255.254",
                "ip_netmask": "255.0.0.0"
            }
        }"#;
        let config: Config<ChFactory> =
            serde_json::from_str(rendered).expect("nix-shaped config must parse");
        config.validate().expect("nix-shaped config must validate");
        assert_eq!(config.worker_id, "worker-1");
        assert_eq!(config.master_addr.to_string(), "192.168.1.10:5000");
        assert_eq!(config.poll_interval_millis.get(), 2000);
        assert_eq!(config.heartbeat_interval_millis.get(), 5000);
    }
}
