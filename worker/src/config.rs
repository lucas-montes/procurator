use std::{net::SocketAddr, num::NonZeroU64, path::Path};

use serde::Deserialize;

use crate::vmm::Factory;

#[derive(Debug, Deserialize)]
pub struct Config<F: Factory> {
    pub listen_addr: SocketAddr,
    pub health_tick_millis: NonZeroU64,
    pub vmm: F::Config,
    #[serde(default)]
    pub proxy: ProxyConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProxyConfig {
    #[serde(default = "default_proxy_addr")]
    pub listen_addr: SocketAddr,
    #[serde(default = "default_enable_tls")]
    pub enable_tls: bool,
    #[serde(default)]
    pub tls_cert_path: Option<String>,
    #[serde(default)]
    pub tls_key_path: Option<String>,
    #[serde(default = "default_external_domain")]
    pub external_domain: String,
}

fn default_proxy_addr() -> SocketAddr {
    "0.0.0.0:8443".parse().unwrap()
}

fn default_enable_tls() -> bool {
    true
}

fn default_external_domain() -> String {
    "localhost".to_string()
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listen_addr: default_proxy_addr(),
            enable_tls: default_enable_tls(),
            tls_cert_path: None,
            tls_key_path: None,
            external_domain: default_external_domain(),
        }
    }
}

impl<F: Factory> Config<F> {
    pub fn from_file(path: impl AsRef<Path> + std::fmt::Debug) -> Self {
        let contents = std::fs::read(&path).unwrap_or_else(|e| {
            tracing::error!(path = ?path, error = %e, "Could not read config");
            std::process::exit(1);
        });

        serde_json::from_slice(&contents).unwrap_or_else(|e| {
            tracing::error!(path = ?path, error = %e, "Failed to parse config");
            std::process::exit(1);
        })
    }
}
