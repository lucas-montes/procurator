use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct AgentsArgs {
    #[command(subcommand)]
    command: AgentsCommands,
}

impl AgentsArgs {
    pub fn execute(self) {
        match self.command {
            AgentsCommands::Up => println!("Bringing up agents..."),
            AgentsCommands::Down => println!("Bringing down agents..."),
            AgentsCommands::Stop => println!("Stopping agents..."),
            AgentsCommands::Start => println!("Starting agents..."),
            AgentsCommands::Restart => println!("Restarting agents..."),
        }
    }
}

#[derive(Debug, Subcommand)]
enum AgentsCommands {
    Up,
    Down,
    Stop,
    Start,
    Restart,
}
