use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug)]
pub enum Error {
    FileMissing,
    RequestFailed(String),
    InvalidCommand(String),
    MissingArgument(String),
    IoError(std::io::Error),
}

/// Procurator CLI
///
/// This CLI is intentionally minimal and declarative.
/// It must never allow commands that introduce drift from the project spec.
///
/// All configuration and environment details come from:
/// - the PROJECT repo flake.nix
/// - declarative Nix files
///
/// The CLI only manipulates:
/// - version control intent
/// - local stack lifecycle
/// - inspection / visualization
///
#[derive(Debug, Parser)]
#[command(name = "procurator", version = "0.0.1")]
#[command(about = "Declarative reproducible developer platform powered by Nix")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

impl Cli {
    pub async fn handle() -> Result<(), Error> {
        let cli = Self::parse();

        match cli.command {
            Commands::Init(args) => {
                super::init::init(args.path);
            }

            Commands::Stack(stack) => match stack.command {
                StackCommands::Up => println!("Stack up"),
                StackCommands::Down => println!("Stack down"),
                StackCommands::Stop => println!("Stop stack"),
                StackCommands::Start => println!("Start stack"),
                StackCommands::Restart => println!("Restart stack"),
            },

            Commands::Test(args) => {
                if let Some(repo) = args.repo {
                    println!("Running tests for repo: {}", repo);
                } else {
                    println!("Running all project tests");
                }
            }

            Commands::Vcs(vcs) => match vcs.command {
                VcsCommands::Clone { identifier } => {
                    println!("Cloning: {}", identifier);
                }
                VcsCommands::Push => println!("Push repos"),
                VcsCommands::Pull => println!("Pull repos"),
            },

            Commands::Inspect => {
                println!("Launching inspection TUI...");
                // TODO: ratatui interface
            }
        };

        Ok(())
    }
}

/// Top-level user commands
#[derive(Debug, Subcommand)]
enum Commands {
    /// Initialize workspace
    ///
    /// Similar to `direnv allow`
    /// - validates flake.nix
    /// - configures local machine
    /// - pulls build cache
    /// - prepares agent
    Init(InitArgs),

    /// Control local project stack lifecycle
    Stack(StackArgs),

    /// Run tests with CI parity
    Test(TestArgs),

    /// Version control integrations
    Vcs(VcsArgs),

    /// Inspect local or remote cluster via TUI
    Inspect,
}

/// Arguments for init command
#[derive(Debug, Args)]
struct InitArgs {
    /// Path to repository (defaults to current directory)
    #[arg(short, long)]
    path: Option<PathBuf>,
}

/// Arguments for stack namespace
///
/// This namespace owns ALL imperative verbs related to execution,
/// but they are safe because they only instantiate declared behavior.
#[derive(Debug, Args)]
struct StackArgs {
    #[command(subcommand)]
    command: StackCommands,
}

/// Declarative stack lifecycle commands
#[derive(Debug, Subcommand)]
enum StackCommands {
    /// Bring the declared stack up locally
    ///
    /// Must launch a TUI log stream aggregating all services
    /// similar to Procfile experience.
    Up,

    /// Full teardown
    Down,

    /// Pause services
    Stop,

    /// Resume services
    Start,

    /// Restart services
    Restart,
}

/// Arguments for test command
#[derive(Debug, Args)]
struct TestArgs {
    /// Specific repo to test
    #[arg(short, long)]
    repo: Option<String>,
}

/// Arguments for vcs namespace
#[derive(Debug, Args)]
struct VcsArgs {
    #[command(subcommand)]
    command: VcsCommands,
}

/// VCS commands are pure intent and never mutate runtime
#[derive(Debug, Subcommand)]
enum VcsCommands {
    /// Clone project configuration or component repos
    Clone { identifier: String },

    /// Push all repos
    Push,

    /// Pull all repos
    Pull,
}
