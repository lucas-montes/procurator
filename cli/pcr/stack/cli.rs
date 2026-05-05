use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct StackArgs {
    #[command(subcommand)]
    command: StackCommands,
}

impl StackArgs {
    pub fn execute(self) {
        match self.command {
            StackCommands::Up => println!("Bringing up the stack..."),
            StackCommands::Down => println!("Bringing down the stack..."),
            StackCommands::Stop => println!("Stopping the stack..."),
            StackCommands::Start => println!("Starting the stack..."),
            StackCommands::Restart => println!("Restarting the stack..."),
        }
    }
}

#[derive(Debug, Subcommand)]
enum StackCommands {
    Up,
    Down,
    Stop,
    Start,
    Restart,
}
