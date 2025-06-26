use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::server::{create, delete};

#[derive(Debug)]
pub enum CliError {
    InvalidCommand(String),
    MissingArgument(String),
    IoError(std::io::Error),
}

#[derive(ValueEnum, Debug, Clone)]
pub enum RessourceType {
    Pod,
    Service,
    Deployment,
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
            Commands::Apply => {
                println!("Applying configuration changes...");
                Ok(())
            }
            Commands::Create(create_cmd) => create_cmd.execute(),
            Commands::Delete(delete_cmd) => delete_cmd.execute(),
            Commands::List(list_cmd) => list_cmd.execute(),
        }
    }
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Apply configuration changes
    Apply,
    /// Create a new resource
    Create(CreateCommand),
    /// Delete an existing resource
    Delete(DeleteCommand),
    /// List resources
    /// TODO: change to monitor
    List(ListCommand),
}

#[derive(Debug, Parser)]
pub struct CreateCommand {
    /// Type of resource to create (pod, service, deployment)
    #[arg(short, long, value_enum)]
    pub resource_type: RessourceType,

    /// Name of the resource
    #[arg(short, long)]
    pub name: String,

    /// Optional file path for resource definition
    #[arg(short, long)]
    pub file: Option<PathBuf>,
}

impl CreateCommand {
    pub fn execute(&self) -> Result<(), CliError> {
        println!("Creating {:?} with name: {}", self.resource_type, self.name);
        if let Some(file_path) = &self.file {
            println!("From file: {:?}", file_path);
        }
        create(self.name.clone());
        Ok(())
    }
}

#[derive(Debug, Parser)]
pub struct DeleteCommand {
    /// Type of resource to delete
    #[arg(short, long, value_enum)]
    pub resource_type: RessourceType,

    /// Name of the resource to delete
    #[arg(short, long)]
    pub name: String,
}

impl DeleteCommand {
    pub fn execute(&self) -> Result<(), CliError> {
        println!("Deleting {:?} with name: {}", self.resource_type, self.name);
        delete(self.name.clone());
        Ok(())
    }
}

#[derive(Debug, Parser)]
pub struct ListCommand {
    /// Type of resource to list (optional, lists all if not specified)
    #[arg(short, long, num_args = 0..)]
    pub resource_type: Option<RessourceType>,
}

impl ListCommand {
    pub fn execute(&self) -> Result<(), CliError> {
        match &self.resource_type {
            Some(resource_type) => println!("Listing all {:?}", resource_type),
            None => println!("Listing all resources"),
        }
        // TODO: Implement list logic
        Ok(())
    }
}
