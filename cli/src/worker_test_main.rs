//! Testing CLI for manually testing the Worker server
//!
//! This binary provides commands to connect to a running Worker server
//! and test the Worker RPC interface (read, listVms, createVm, deleteVm).

use clap::{Parser, Subcommand};
use std::net::SocketAddr;

mod worker_testing;

#[derive(Debug, Parser)]
#[command(name = "pcr-worker-test", version = "0.1.0")]
#[command(about = "Test client for Procurator Worker server")]
struct Cli {
    /// Worker server address (e.g., 127.0.0.1:6000)
    #[arg(short, long, default_value = "127.0.0.1:6000")]
    addr: SocketAddr,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Fetch worker status
    Read,

    /// List VMs
    ListVms,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .init();

    let cli = Cli::parse();

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            match cli.command {
                Commands::Read => worker_testing::test_worker_read(cli.addr).await?,
                Commands::ListVms => worker_testing::test_list_vms(cli.addr).await?,
            }

            Ok(())
        })
        .await
}
