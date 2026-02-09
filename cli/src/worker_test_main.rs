//! Testing CLI for manually testing the Worker server
//!
//! This binary provides commands to connect to a running Worker server
//! and test the Worker RPC interface and nested Vm interface.

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

    /// Get a VM capability and exercise VM methods
    Vm {
        #[command(subcommand)]
        command: VmCommands,
    },
}

#[derive(Debug, Subcommand)]
enum VmCommands {
    /// Read VM status
    Read {
        /// VM id
        #[arg(short, long)]
        vm_id: String,
    },

    /// Get VM logs
    Logs {
        /// VM id
        #[arg(short, long)]
        vm_id: String,

        /// Follow logs
        #[arg(long, default_value_t = false)]
        follow: bool,

        /// Tail lines
        #[arg(long, default_value_t = 100)]
        tail_lines: u32,
    },

    /// Execute a command inside VM
    Exec {
        /// VM id
        #[arg(short, long)]
        vm_id: String,

        /// Command to execute
        #[arg(short, long)]
        command: String,

        /// Command arguments
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },

    /// Get connection info
    ConnectionInfo {
        /// VM id
        #[arg(short, long)]
        vm_id: String,
    },
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
                Commands::Vm { command } => match command {
                    VmCommands::Read { vm_id } => {
                        worker_testing::test_vm_read(cli.addr, vm_id).await?
                    }
                    VmCommands::Logs {
                        vm_id,
                        follow,
                        tail_lines,
                    } => {
                        worker_testing::test_vm_logs(cli.addr, vm_id, follow, tail_lines).await?
                    }
                    VmCommands::Exec { vm_id, command, args } => {
                        worker_testing::test_vm_exec(cli.addr, vm_id, command, args).await?
                    }
                    VmCommands::ConnectionInfo { vm_id } => {
                        worker_testing::test_vm_connection_info(cli.addr, vm_id).await?
                    }
                },
            }

            Ok(())
        })
        .await
}
