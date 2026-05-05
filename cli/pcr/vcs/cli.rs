use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct VcsArgs {
    #[command(subcommand)]
    command: VcsSubCommands,
}

impl VcsArgs {
    pub fn execute(self) {
        match self.command {
            VcsSubCommands::Repo(repo_args) => match repo_args.command {
                RepoCommands::Push => println!("Pushing repository..."),
                RepoCommands::Pull => println!("Pulling repository..."),
            },
            VcsSubCommands::Project(project_args) => match project_args.command {
                ProjectCommands::Push => println!("Pushing project..."),
                ProjectCommands::Pull => println!("Pulling project..."),
            },
        }
    }
}

#[derive(Debug, Subcommand)]
enum VcsSubCommands {
    Repo(RepoArgs),
    Project(ProjectArgs),
}

#[derive(Debug, Args)]
struct RepoArgs {
    #[command(subcommand)]
    command: RepoCommands,
}
#[derive(Debug, Subcommand)]
enum RepoCommands {
    Push,
    Pull,
}

#[derive(Debug, Args)]
struct ProjectArgs {
    #[command(subcommand)]
    command: ProjectCommands,
}
#[derive(Debug, Subcommand)]
enum ProjectCommands {
    Push,
    Pull,
}
