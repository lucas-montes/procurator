mod agents;
mod init;
mod stack;
mod vcs;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::{agents::AgentsArgs, stack::StackArgs, vcs::VcsArgs};

#[derive(Debug)]
enum Error {
    FileMissing,
    RequestFailed(String),
    InvalidCommand(String),
    MissingArgument(String),
    IoError(std::io::Error),
}

/// Procurator CLI
#[derive(Debug, Parser)]
#[command(name = "procurator", version = "0.0.1")]
#[command(about = "Declarative reproducible developer platform powered by Nix")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Top-level user commands
#[derive(Debug, Subcommand)]
enum Commands {
    /// Initialize workspace
    Init(InitArgs),

    /// Control local project stack lifecycle
    Stack(StackArgs),

    /// Manage projects and repositories
    Vcs(VcsArgs),

    /// Workspace management for AI agents
    Agents(AgentsArgs),
}

#[derive(Debug, Args)]
struct InitArgs {
    /// Path to repository (defaults to current directory)
    #[arg(short, long)]
    path: Option<PathBuf>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();
    match cli.command {
        Commands::Init(args) => {
            init::init(args.path);
        }

        Commands::Stack(args) => {
            args.execute().await;
        }
        Commands::Agents(args) => {
            args.execute();
        }
        Commands::Vcs(args) => {
            args.execute();
        }
    };
}
