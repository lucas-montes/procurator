//! Testing CLI for manually testing the Master server
//!
//! This binary provides commands to connect to a running Master server
//! and test each of the RPC interfaces (State, Worker, Control)

use clap::{Parser, Subcommand};
use std::net::SocketAddr;

mod testing;

#[derive(Debug, Parser)]
#[command(name = "pcr-test", version = "0.1.0")]
#[command(about = "Test client for Procurator Master server")]
struct Cli {
    /// Master server address (e.g., 127.0.0.1:5000)
    #[arg(short, long, default_value = "127.0.0.1:5000")]
    addr: SocketAddr,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Test Master.State interface (CD platform operations)
    State {
        #[command(subcommand)]
        command: StateCommands,
    },

    /// Test Master.Worker interface (worker synchronization)
    Worker {
        #[command(subcommand)]
        command: WorkerCommands,
    },

    /// Test Master.Control interface (CLI inspection)
    Control {
        #[command(subcommand)]
        command: ControlCommands,
    },
}

#[derive(Debug, Subcommand)]
enum StateCommands {
    /// Publish a new generation to the master
    Publish {
        /// Git commit hash
        #[arg(short, long)]
        commit: String,

        /// Generation number
        #[arg(short, long)]
        generation: u64,

        /// Intent hash
        #[arg(short, long)]
        intent_hash: String,

        /// Number of test VMs to create
        #[arg(short = 'n', long, default_value = "2")]
        num_vms: usize,
    },
}

#[derive(Debug, Subcommand)]
enum WorkerCommands {
    /// Get assignment for a worker
    GetAssignment {
        /// Worker ID
        #[arg(short, long)]
        worker_id: String,

        /// Last seen generation
        #[arg(short, long, default_value = "0")]
        last_seen_generation: u64,
    },

    /// Push worker data to master
    PushData {
        /// Worker ID
        #[arg(short, long)]
        worker_id: String,

        /// Observed generation
        #[arg(short, long)]
        observed_generation: u64,

        /// Number of running VMs to report
        #[arg(short = 'n', long, default_value = "1")]
        num_vms: usize,
    },
}

#[derive(Debug, Subcommand)]
enum ControlCommands {
    /// Get cluster status
    Status,

    /// Get a specific worker capability
    GetWorker {
        /// Worker ID to retrieve
        #[arg(short, long)]
        worker_id: String,
    },

    /// Get a specific VM capability
    GetVm {
        /// VM ID to retrieve
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
    local.run_until(async move {
        match cli.command {
            Commands::State { command } => match command {
                StateCommands::Publish {
                    commit,
                    generation,
                    intent_hash,
                    num_vms,
                } => {
                    testing::test_publish(
                        cli.addr,
                        commit,
                        generation,
                        intent_hash,
                        num_vms,
                    ).await?;
                }
            },

            Commands::Worker { command } => match command {
                WorkerCommands::GetAssignment {
                    worker_id,
                    last_seen_generation,
                } => {
                    testing::test_get_assignment(
                        cli.addr,
                        worker_id,
                        last_seen_generation,
                    ).await?;
                }

                WorkerCommands::PushData {
                    worker_id,
                    observed_generation,
                    num_vms,
                } => {
                    testing::test_push_data(
                        cli.addr,
                        worker_id,
                        observed_generation,
                        num_vms,
                    ).await?;
                }
            },

            Commands::Control { command } => match command {
                ControlCommands::Status => {
                    testing::test_get_cluster_status(cli.addr).await?;
                }

                ControlCommands::GetWorker { worker_id } => {
                    testing::test_get_worker(cli.addr, worker_id).await?;
                }

                ControlCommands::GetVm { vm_id } => {
                    testing::test_get_vm(cli.addr, vm_id).await?;
                }
            },
        }

        Ok(())
    }).await
}
