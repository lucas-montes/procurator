use clap::Subcommand;

#[derive(Debug, Subcommand)]
/// Subcommands for `pcr stack`
pub enum StackCommands {
    /// Start all services
    Up,
    /// Stop all services
    Down,
    /// Stop without removing state
    Stop,
    /// Start stopped services
    Start,
    /// Restart all services
    Restart,
}
