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
    pub proxy: ProxyConfig,
    pub vmm: F::Config,
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
        self.proxy.validate(self.rpc_listen_addr)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProxyConfig {
    pub opencode_upstream_port: u16,
    pub public_listen_addr: SocketAddr,
    pub tls_cert_path: PathBuf,
    pub tls_key_path: PathBuf,
    pub jwt_hs256_secret: String,
    pub timeouts: ProxyTimeouts,
}

impl ProxyConfig {
    fn validate(&self, rpc_listen_addr: SocketAddr) -> Result<(), ConfigError> {
        if self.public_listen_addr == rpc_listen_addr {
            return Err(ConfigError::ListenerConflict {
                rpc_listen_addr,
                proxy_listen_addr: self.public_listen_addr,
            });
        }

        validate_path_exists(&self.tls_cert_path, "proxy.tls_cert_path")?;
        validate_path_exists(&self.tls_key_path, "proxy.tls_key_path")?;

        if self.jwt_hs256_secret.trim().is_empty() {
            return Err(ConfigError::EmptyProxyJwtSecret);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProxyTimeouts {
    pub upstream_connect_timeout_millis: Option<NonZeroU64>,
    pub upstream_request_timeout_millis: Option<NonZeroU64>,
}

#[derive(Debug)]
enum ConfigError {
    ListenerConflict {
        rpc_listen_addr: SocketAddr,
        proxy_listen_addr: SocketAddr,
    },
    MissingPath {
        field: &'static str,
        path: PathBuf,
    },
    EmptyProxyJwtSecret,
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
            ConfigError::MissingPath { field, path } => {
                write!(f, "{field} does not exist: {}", path.display())
            }
            ConfigError::EmptyProxyJwtSecret => {
                write!(f, "proxy.jwt_hs256_secret must not be empty")
            }
        }
    }
}

fn validate_path_exists(path: &Path, field: &'static str) -> Result<(), ConfigError> {
    if path.exists() {
        Ok(())
    } else {
        Err(ConfigError::MissingPath {
            field,
            path: path.to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, path::PathBuf};

    use super::{ProxyConfig, ProxyTimeouts};

    fn proxy_config() -> ProxyConfig {
        ProxyConfig {
            opencode_upstream_port: 4096,
            public_listen_addr: "0.0.0.0:8443".parse().expect("valid socket addr"),
            tls_cert_path: PathBuf::from("/tmp/procurator2-worker-test-cert.pem"),
            tls_key_path: PathBuf::from("/tmp/procurator2-worker-test-key.pem"),
            jwt_hs256_secret: "super-secret".to_string(),
            timeouts: ProxyTimeouts::default(),
        }
    }

    fn create_temp_file(path: &PathBuf) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create test directory");
        }

        std::fs::write(path, "test").expect("write temp file");
    }

    #[test]
    fn rejects_same_listener_for_rpc_and_proxy() {
        let mut proxy = proxy_config();
        let addr: SocketAddr = "127.0.0.1:8443".parse().expect("valid socket addr");
        proxy.public_listen_addr = addr;

        create_temp_file(&proxy.tls_cert_path);
        create_temp_file(&proxy.tls_key_path);

        let result = proxy.validate(addr);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_missing_tls_paths() {
        let mut proxy = proxy_config();
        proxy.tls_cert_path = PathBuf::from("/tmp/procurator2-missing-cert.pem");
        proxy.tls_key_path = PathBuf::from("/tmp/procurator2-missing-key.pem");

        let rpc_addr: SocketAddr = "127.0.0.1:8080".parse().expect("valid socket addr");
        let result = proxy.validate(rpc_addr);

        assert!(result.is_err());
    }

    #[test]
    fn rejects_empty_jwt_secret() {
        let mut proxy = proxy_config();
        proxy.jwt_hs256_secret = "   ".to_string();

        create_temp_file(&proxy.tls_cert_path);
        create_temp_file(&proxy.tls_key_path);

        let rpc_addr: SocketAddr = "127.0.0.1:8080".parse().expect("valid socket addr");
        let result = proxy.validate(rpc_addr);

        assert!(result.is_err());
    }
}
