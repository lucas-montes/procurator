use clap::{Parser, Subcommand, ValueEnum};


#[derive(Debug)]
pub enum CliError {
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
    pub fn handle() -> Result<(), CliError> {
        let cli = Cli::parse();

        match cli.command {
            Commands::Apply(cmd) => cmd.execute(),
            Commands::List(cmd) => cmd.execute(),
        }
    }
}

#[derive(Debug, Subcommand)]
enum Commands {
    Apply,
    /// Apply configuration changes to an area
    Apply(Apply),
    /// List resources
    List(List),
}

#[derive(ValueEnum, Debug, Clone)]
enum Area {
    Staging,
    Prod,
}

#[derive(Debug, Parser)]
struct Apply {
    /// The area to deploy to
    #[arg(short, long, value_enum)]
    resource_type: Area,

    #[arg(short, long)]
    name: String,
}

impl Apply {
    fn execute(&self) -> Result<(), CliError> {
        Ok(())
    }
}

#[derive(Debug, Parser)]
struct List {}

impl List {
    fn execute(&self) -> Result<(), CliError> {
        Ok(())
    }
}
