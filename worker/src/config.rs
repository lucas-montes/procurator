use std::{
    net::SocketAddr,
    num::NonZeroU64,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::vmm::Factory;

#[derive(Debug, Deserialize)]
pub struct CloudHypervisorSection {
    binary_path: PathBuf,
    socket_dir: PathBuf,
    socket_timeout_secs: u64,
    bridge_name: String,
}

#[derive(Debug, Deserialize)]
pub struct Config<F: Factory> {
    pub listen_addr: SocketAddr,
    master_addr: SocketAddr,
    pub health_tick_millis: NonZeroU64,
    pub vmm: F::Config,
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
