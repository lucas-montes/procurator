use std::{net::SocketAddr, path::PathBuf};

use serde::Deserialize;


#[derive(Debug, Deserialize)]
pub struct CloudHypervisorSection {
    binary_path: PathBuf,
    socket_dir: PathBuf,
    socket_timeout_secs: u64,
    bridge_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    listen_addr: SocketAddr,
    master_addr: SocketAddr,
    cloud_hypervisor: CloudHypervisorSection,
}
