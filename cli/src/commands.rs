use std::{path::PathBuf, time::Duration};

use clap::{Parser, Subcommand, ValueEnum};

use crate::{client::Client, interactive};

#[derive(Debug)]
pub enum CliError {
    FileMissing,
    RequestFailed(String),
    InvalidCommand(String),
    MissingArgument(String),
    IoError(std::io::Error),
}

#[derive(Debug, Parser)]
#[command(name = "Prom", version = "0.0.1")]
#[command(about = "Manage your cluster")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

impl Cli {
    pub async fn handle() -> Result<(), CliError> {
        let cli = Cli::parse();
        let client = Client::new("127.0.0.1:3000".parse().unwrap()).await;

        match cli.command {
            Commands::Apply(cmd) => cmd.execute(client).await,
            Commands::List(cmd) => cmd.execute(client).await,
            Commands::Monitor(cmd) => cmd.execute(client).await,
        }
    }
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Apply configuration changes of a specific file
    Apply(Apply),
    /// List resources
    List(List),
    /// Open an interactive dashboard to monitor the cluster
    Monitor(Monitor),
}

trait CommandExt {
    async fn execute(&self, client: Client) -> Result<(), CliError>;
}

#[derive(ValueEnum, Debug, Clone)]
pub enum Area {
    Staging,
    Prod,
}

#[derive(Debug, Parser)]
struct Apply {
    /// The file to use as configuration for the cluster
    #[arg(short, long)]
    config_file: PathBuf,
}

impl CommandExt for Apply {
    /// Will send the file
    async fn execute(&self, client: Client) -> Result<(), CliError> {
        let config_file = self
            .config_file
            .as_path()
            .to_str()
            .ok_or(CliError::FileMissing)?;
        client.apply(config_file).await
    }
}

#[derive(Debug, Parser)]
struct List {}

impl CommandExt for List {
    async fn execute(&self, client: Client) -> Result<(), CliError> {
        Ok(())
    }
}
#[derive(Debug, Parser)]
struct Monitor {
    /// time in ms between two ticks.
    #[arg(short, long, default_value_t = 250)]
    tick_rate: u64,

    /// whether unicode symbols are used to improve the overall look of the app
    #[arg(short, long, default_value_t = true)]
    unicode: bool,
}
impl CommandExt for Monitor {
    async fn execute(&self, client: Client) -> Result<(), CliError> {
        client.monitor().await?;
        let tick_rate = Duration::from_millis(self.tick_rate);
        interactive::run(tick_rate, self.unicode).map_err(CliError::IoError)
    }
}
