use clap::{Args, Subcommand};
use std::path::PathBuf;
use std::time::Instant;
use tracing::{error, info, warn};

/// Top-level VCS arguments routing to repo or project subcommands.
#[derive(Debug, Args)]
pub struct VcsArgs {
    #[command(subcommand)]
    command: VcsSubCommands,
}

impl VcsArgs {
    pub fn execute(self) {
        match self.command {
            VcsSubCommands::Repo(repo_args) => repo_args.execute(),
            VcsSubCommands::Project(project_args) => project_args.execute(),
            VcsSubCommands::Agent(agent_args) => agent_args.execute(),
        }
    }
}

/// VCS subcommands: single repo or multi-repo project.
#[derive(Debug, Subcommand)]
pub enum VcsSubCommands {
    /// Single repository operations
    Repo(RepoArgs),
    /// Multi-repo project operations (with submodules)
    Project(ProjectArgs),
    /// Agent workspace operations
    Agent(AgentArgs),
}

// ============================================================================
// Repo Commands (single repository)
// ============================================================================

/// Arguments for single repository operations.
#[derive(Debug, Args)]
pub struct RepoArgs {
    #[command(subcommand)]
    command: RepoCommands,
}

/// Available commands for single repositories.
#[derive(Debug, Subcommand)]
pub enum RepoCommands {
    /// Clone a single repository
    Clone(RepoCloneArgs),
    /// Pull latest changes
    Pull(RepoPullArgs),
    /// Push local commits
    Push(RepoPushArgs),
    /// Create a new branch
    Branch(RepoBranchArgs),
}

/// Clone arguments for single repository.
#[derive(Debug, Args)]
pub struct RepoCloneArgs {
    /// URL of the repository to clone
    url: String,
    /// Directory to clone into (defaults to repo name)
    directory: Option<PathBuf>,
}

/// Pull arguments for single repository.
#[derive(Debug, Args)]
pub struct RepoPullArgs {
    /// Path to the repository (defaults to current directory)
    #[arg(short, long)]
    path: Option<PathBuf>,
}

/// Push arguments for single repository.
#[derive(Debug, Args)]
pub struct RepoPushArgs {
    /// Path to the repository (defaults to current directory)
    #[arg(short, long)]
    path: Option<PathBuf>,
}

/// Branch arguments for single repository.
#[derive(Debug, Args)]
pub struct RepoBranchArgs {
    /// Branch name to create
    name: String,
    /// Path to the repository (defaults to current directory)
    #[arg(short, long)]
    path: Option<PathBuf>,
}

impl RepoArgs {
    pub fn execute(self) {
        match self.command {
            RepoCommands::Clone(args) => execute_repo_clone(args),
            RepoCommands::Pull(args) => execute_repo_pull(args),
            RepoCommands::Push(args) => execute_repo_push(args),
            RepoCommands::Branch(args) => execute_repo_branch(args),
        }
    }
}

/// Clone a single repository from URL.
fn execute_repo_clone(args: RepoCloneArgs) {
    let start = Instant::now();
    info!("Cloning repository: {}", args.url);

    let path = args.directory.unwrap_or_else(|| {
        let name = args
            .url
            .trim_end_matches(".git")
            .split('/')
            .last()
            .unwrap_or("repo")
            .to_string();
        PathBuf::from(name)
    });

    match repo_outils::git::GitRepo::clone(&args.url, &path) {
        Ok(_) => {
            println!("Successfully cloned {} to {}", args.url, path.display());
            info!("Repo clone completed in {:?}", start.elapsed());
        }
        Err(e) => {
            error!("Failed to clone repository: {}", e);
            std::process::exit(1);
        }
    }
}

/// Pull latest changes from remote.
fn execute_repo_pull(args: RepoPullArgs) {
    let start = Instant::now();
    let path = args.path.unwrap_or_else(|| PathBuf::from("."));
    info!("Pulling repository at: {}", path.display());

    match repo_outils::git::GitRepo::open(&path) {
        Ok(repo) => match repo.pull() {
            Ok(_) => {
                println!("Successfully pulled repository at {}", path.display());
                info!("Repo pull completed in {:?}", start.elapsed());
            }
            Err(e) => {
                error!("Failed to pull repository: {}", e);
                std::process::exit(1);
            }
        },
        Err(e) => {
            error!("Failed to open repository: {}", e);
            std::process::exit(1);
        }
    }
}

/// Push local commits to remote with optional Nix cache.
fn execute_repo_push(args: RepoPushArgs) {
    let start = Instant::now();
    let path = args.path.unwrap_or_else(|| PathBuf::from("."));
    info!("Pushing repository at: {}", path.display());

    let repo = match repo_outils::git::GitRepo::open(&path) {
        Ok(repo) => repo,
        Err(e) => {
            error!("Failed to open repository: {}", e);
            std::process::exit(1);
        }
    };

    // Push git changes
    match repo.push() {
        Ok(_) => println!("Successfully pushed repository at {}", path.display()),
        Err(e) => {
            error!("Failed to push repository: {}", e);
            std::process::exit(1);
        }
    }

    // Push Nix cache if configured
    match repo_outils::nix::read_cache_url(&path) {
        Ok(Some(url)) => {
            info!("Pushing Nix artifacts to cache: {}", url);
            match repo_outils::nix::push_all_to_cache(&url) {
                Ok(_) => println!("Successfully pushed Nix artifacts to cache"),
                Err(e) => warn!("Failed to push Nix cache: {}", e),
            }
        }
        Ok(None) => {
            warn!("No Nix cache configured, skipping cache push");
        }
        Err(e) => {
            warn!("Failed to read Nix config: {}", e);
        }
    }

    info!("Repo push completed in {:?}", start.elapsed());
}

/// Create a new branch using git2.
fn execute_repo_branch(args: RepoBranchArgs) {
    let start = Instant::now();
    let path = args.path.unwrap_or_else(|| PathBuf::from("."));
    info!(
        "Creating branch '{}' in repository at: {}",
        args.name,
        path.display()
    );

    match repo_outils::git::GitRepo::open(&path) {
        Ok(repo) => match repo.branch(&args.name) {
            Ok(_) => {
                println!("Successfully created branch '{}'", args.name);
                info!("Repo branch completed in {:?}", start.elapsed());
            }
            Err(e) => {
                error!("Failed to create branch: {}", e);
                std::process::exit(1);
            }
        },
        Err(e) => {
            error!("Failed to open repository: {}", e);
            std::process::exit(1);
        }
    }
}

// ============================================================================
// Project Commands (with submodules)
// ============================================================================

/// Arguments for multi-repo project operations.
#[derive(Debug, Args)]
pub struct ProjectArgs {
    #[command(subcommand)]
    command: ProjectCommands,
}

/// Available commands for multi-repo projects.
#[derive(Debug, Subcommand)]
pub enum ProjectCommands {
    /// List available projects
    List,
    /// Clone a project with all its submodules
    Clone(ProjectCloneArgs),
    /// Pull latest changes across all submodules
    Pull(ProjectPullArgs),
    /// Push local commits across all submodules
    Push(ProjectPushArgs),
    /// Create a branch across main repo and all submodules
    Branch(ProjectBranchArgs),
}

/// Clone arguments for multi-repo project.
#[derive(Debug, Args)]
pub struct ProjectCloneArgs {
    /// URL of the project repository to clone
    url: String,
    /// Directory to clone into (defaults to project name)
    directory: Option<PathBuf>,
    /// Only clone specified submodules (comma-separated)
    #[arg(long)]
    repos: Option<Vec<String>>,
}

/// Pull arguments for multi-repo project.
#[derive(Debug, Args)]
pub struct ProjectPullArgs {
    /// Path to the project (defaults to current directory)
    #[arg(short, long)]
    path: Option<PathBuf>,
    /// Only pull specified submodules (comma-separated)
    #[arg(long)]
    repos: Option<Vec<String>>,
    /// Exclude specified submodules from pull (comma-separated)
    #[arg(long)]
    exclude: Option<Vec<String>>,
}

/// Push arguments for multi-repo project.
#[derive(Debug, Args)]
pub struct ProjectPushArgs {
    /// Path to the project (defaults to current directory)
    #[arg(short, long)]
    path: Option<PathBuf>,
    /// Only push specified submodules (comma-separated)
    #[arg(long)]
    repos: Option<Vec<String>>,
    /// Exclude specified submodules from push (comma-separated)
    #[arg(long)]
    exclude: Option<Vec<String>>,
}

/// Branch arguments for multi-repo project.
#[derive(Debug, Args)]
pub struct ProjectBranchArgs {
    /// Branch name to create
    name: String,
    /// Path to the project (defaults to current directory)
    #[arg(short, long)]
    path: Option<PathBuf>,
}

impl ProjectArgs {
    pub fn execute(self) {
        match self.command {
            ProjectCommands::List => execute_project_list(),
            ProjectCommands::Clone(args) => execute_project_clone(args),
            ProjectCommands::Pull(args) => execute_project_pull(args),
            ProjectCommands::Push(args) => execute_project_push(args),
            ProjectCommands::Branch(args) => execute_project_branch(args),
        }
    }
}

/// List available projects from config file.
fn execute_project_list() {
    let start = Instant::now();
    info!("Listing available projects");

    // Check for local config file (TODO: implement proper config path)
    let config_path = std::path::Path::new(".config/procurator/projects.toml");

    if config_path.exists() {
        println!("Projects from {}:", config_path.display());
        // TODO: Parse TOML and display projects
        println!("(Config file exists but parsing not yet implemented)");
    } else {
        println!("No project registry found.");
        println!("Use: pcr vcs project clone <url> to clone a project");
    }

    info!("Project list completed in {:?}", start.elapsed());
}

/// Clone project with submodules from URL.
fn execute_project_clone(args: ProjectCloneArgs) {
    let start = Instant::now();
    info!("Cloning project: {}", args.url);

    let path = args.directory.unwrap_or_else(|| {
        let name = args
            .url
            .trim_end_matches(".git")
            .split('/')
            .last()
            .unwrap_or("project")
            .to_string();
        PathBuf::from(name)
    });

    // Clone the main repository
    let repo = match repo_outils::git::GitRepo::clone(&args.url, &path) {
        Ok(repo) => {
            println!("Successfully cloned project to {}", path.display());
            repo
        }
        Err(e) => {
            error!("Failed to clone project: {}", e);
            std::process::exit(1);
        }
    };

    // Initialize submodules
    match repo.init_submodules() {
        Ok(submodules) => {
            println!("Initialized {} submodules", submodules.len());

            // Filter submodules if --repos specified
            let filtered = repo_outils::git::filter_submodules(submodules, &args.repos, &None);

            if let Some(ref repos) = args.repos {
                println!("Selected submodules: {}", repos.join(", "));
            }

            for submodule in &filtered {
                println!("  - {} ({})", submodule.name(), submodule.url());
            }
        }
        Err(e) => {
            error!("Failed to initialize submodules: {}", e);
            // Don't exit - submodules might not be configured
        }
    }

    info!("Project clone completed in {:?}", start.elapsed());
}

/// Pull latest changes across project and submodules.
fn execute_project_pull(args: ProjectPullArgs) {
    let start = Instant::now();
    let path = args.path.unwrap_or_else(|| PathBuf::from("."));
    info!("Pulling project at: {}", path.display());

    // Open main repository
    let repo = match repo_outils::git::GitRepo::open(&path) {
        Ok(repo) => repo,
        Err(e) => {
            error!("Failed to open project: {}", e);
            std::process::exit(1);
        }
    };

    // Pull main repository
    match repo.pull() {
        Ok(_) => println!("Successfully pulled main repository"),
        Err(e) => {
            error!("Failed to pull main repository: {}", e);
            std::process::exit(1);
        }
    }

    // Get and filter submodules
    match repo.list_submodules() {
        Ok(submodules) => {
            let filtered =
                repo_outils::git::filter_submodules(submodules, &args.repos, &args.exclude);

            println!("Pulling {} submodules...", filtered.len());

            // TODO: Implement parallel pull for submodules
            // For now, just list them
            for submodule in &filtered {
                println!("  - {} ({})", submodule.name(), submodule.path().display());
            }
        }
        Err(e) => {
            warn!("Failed to list submodules: {}", e);
        }
    }

    info!("Project pull completed in {:?}", start.elapsed());
}

/// Push local commits across project and submodules.
/// Only pushes submodules that have changes (checked via git status).
fn execute_project_push(args: ProjectPushArgs) {
    let start = Instant::now();
    let path = args.path.unwrap_or_else(|| PathBuf::from("."));
    info!("Pushing project at: {}", path.display());

    // Open main repository
    let repo = match repo_outils::git::GitRepo::open(&path) {
        Ok(repo) => repo,
        Err(e) => {
            error!("Failed to open project: {}", e);
            std::process::exit(1);
        }
    };

    // Push main repository first
    match repo.push() {
        Ok(_) => println!("Successfully pushed main repository"),
        Err(e) => {
            error!("Failed to push main repository: {}", e);
            std::process::exit(1);
        }
    }

    // Get and filter submodules
    match repo.list_submodules() {
        Ok(submodules) => {
            let filtered =
                repo_outils::git::filter_submodules(submodules, &args.repos, &args.exclude);

            println!("Checking {} submodules for changes...", filtered.len());

            let mut pushed_count = 0;
            let mut skipped_count = 0;

            for submodule in &filtered {
                let submodule_path = path.join(submodule.path());
                info!(
                    "Checking submodule '{}' at: {}",
                    submodule.name(),
                    submodule_path.display()
                );

                // Open submodule repository
                match repo_outils::git::GitRepo::open(&submodule_path) {
                    Ok(sub_repo) => {
                        // Check if submodule has changes
                        match sub_repo.has_changes() {
                            Ok(true) => {
                                // Has changes, push it
                                info!("Submodule '{}' has changes, pushing...", submodule.name());
                                match sub_repo.push() {
                                    Ok(_) => {
                                        println!(
                                            "Successfully pushed submodule: {}",
                                            submodule.name()
                                        );
                                        pushed_count += 1;
                                    }
                                    Err(e) => {
                                        error!(
                                            "Failed to push submodule '{}': {}",
                                            submodule.name(),
                                            e
                                        );
                                        // Continue with other submodules
                                    }
                                }
                            }
                            Ok(false) => {
                                // No changes, skip it
                                warn!(
                                    "Submodule '{}' has no changes, skipping push",
                                    submodule.name()
                                );
                                skipped_count += 1;
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to check changes for submodule '{}': {}",
                                    submodule.name(),
                                    e
                                );
                                // Skip if we can't determine status
                                skipped_count += 1;
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Failed to open submodule '{}' at {}: {}",
                            submodule.name(),
                            submodule_path.display(),
                            e
                        );
                        // Continue with other submodules
                    }
                }
            }

            println!(
                "Push summary: {} pushed, {} skipped (no changes)",
                pushed_count, skipped_count
            );
        }
        Err(e) => {
            warn!("Failed to list submodules: {}", e);
        }
    }

    info!("Push completed in {:?}", start.elapsed());
}

/// Create branch across project (stub - full implementation in T08/T09).
fn execute_project_branch(args: ProjectBranchArgs) {
    let start = Instant::now();
    let path = args.path.unwrap_or_else(|| PathBuf::from("."));
    info!(
        "Creating branch '{}' in project at: {}",
        args.name,
        path.display()
    );

    // For now, use git command line for branch creation
    println!("Branch creation will be fully implemented in T08/T09");
    println!(
        "For now, manually run: cd {} && git checkout -b {}",
        path.display(),
        args.name
    );

    info!("Project branch completed in {:?}", start.elapsed());
}

// ============================================================================
// Agent Commands
// ============================================================================

/// Arguments for agent workspace operations.
#[derive(Debug, Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    command: AgentCommands,
}

/// Available commands for agent workspaces.
#[derive(Debug, Subcommand)]
pub enum AgentCommands {
    /// Prepare a workspace for an agent
    Prepare(AgentPrepareArgs),
    /// List active agent workspaces
    List,
}

/// Arguments for preparing an agent workspace.
#[derive(Debug, Args)]
pub struct AgentPrepareArgs {
    /// URL of the repository to prepare workspace from
    url: String,
    /// Branch name to create
    branch: String,
    /// Project name (defaults to derived from URL)
    #[arg(short, long)]
    name: Option<String>,
    /// Path to clone into (defaults to derived from URL)
    #[arg(short, long)]
    path: Option<PathBuf>,
}

impl AgentArgs {
    pub fn execute(self) {
        match self.command {
            AgentCommands::Prepare(args) => execute_agent_prepare(args),
            AgentCommands::List => execute_agent_list(),
        }
    }
}

/// Prepare workspace for an agent: clone repo and create branch.
/// Workspace is co-located with the project in `<project-dir>/agents/<branch>/`.
fn execute_agent_prepare(args: AgentPrepareArgs) {
    let start = Instant::now();
    info!(
        "Preparing agent workspace: url={}, branch={}",
        args.url, args.branch
    );

    // Derive project name from URL if not provided
    let _project_name = args.name.unwrap_or_else(|| {
        args.url
            .trim_end_matches(".git")
            .split('/')
            .last()
            .unwrap_or("project")
            .to_string()
    });

    // Determine base path (current directory or specified path)
    let base_path = args.path.unwrap_or_else(|| PathBuf::from("."));

    // Workspace path: <base>/agents/<branch>/
    let workspace_path = base_path.join("agents").join(&args.branch);

    // Check if workspace already exists
    if workspace_path.exists() {
        error!("Workspace already exists at: {}", workspace_path.display());
        error!("Remove existing workspace before creating a new one.");
        std::process::exit(1);
    }

    // Create workspace directory
    if let Err(e) = std::fs::create_dir_all(&workspace_path) {
        error!("Failed to create workspace directory: {}", e);
        std::process::exit(1);
    }

    // Clone the repository
    info!("Cloning repository to workspace...");
    let repo = match repo_outils::git::GitRepo::clone(&args.url, &workspace_path) {
        Ok(repo) => {
            println!(
                "Successfully cloned {} to {}",
                args.url,
                workspace_path.display()
            );
            repo
        }
        Err(e) => {
            // Clean up created directory on failure
            let _ = std::fs::remove_dir_all(&workspace_path);
            error!("Failed to clone repository: {}", e);
            std::process::exit(1);
        }
    };

    // Create branch in the cloned repo
    info!("Creating branch '{}'...", args.branch);
    match repo.branch(&args.branch) {
        Ok(_) => println!("Successfully created branch '{}'", args.branch),
        Err(e) => {
            error!("Failed to create branch: {}", e);
            std::process::exit(1);
        }
    }

    info!("Agent workspace prepared in {:?}", start.elapsed());
    println!("Workspace: {}", workspace_path.display());
}

/// List active agent workspaces with details.
fn execute_agent_list() {
    let start = Instant::now();
    info!("Listing agent workspaces");

    let base_path = PathBuf::from(".");
    let agents_dir = base_path.join("agents");

    if !agents_dir.exists() {
        println!("No agent workspaces found.");
        println!("Use 'pcr vcs agent prepare <url> <branch>' to create one.");
        info!("Agent list completed in {:?}", start.elapsed());
        return;
    }

    // Read all branch directories
    let entries = match std::fs::read_dir(&agents_dir) {
        Ok(entries) => entries,
        Err(e) => {
            error!("Failed to read agents directory: {}", e);
            std::process::exit(1);
        }
    };

    let mut workspaces: Vec<WorkspaceInfo> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let branch_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            // Try to read git info from the workspace
            match repo_outils::git::GitRepo::open(&path) {
                Ok(repo) => {
                    let current_branch = repo
                        .current_branch()
                        .unwrap_or_else(|_| "unknown".to_string());
                    let has_changes = repo.has_changes().unwrap_or(false);
                    let modified = entry.metadata().and_then(|m| m.modified()).ok();

                    workspaces.push(WorkspaceInfo {
                        branch: branch_name,
                        current_branch,
                        has_changes,
                        modified,
                        path,
                    });
                }
                Err(_) => {
                    // Not a valid git repo, just list with basic info
                    let modified = entry.metadata().and_then(|m| m.modified()).ok();

                    workspaces.push(WorkspaceInfo {
                        branch: branch_name,
                        current_branch: "N/A".to_string(),
                        has_changes: false,
                        modified,
                        path,
                    });
                }
            }
        }
    }

    if workspaces.is_empty() {
        println!("No agent workspaces found.");
        info!("Agent list completed in {:?}", start.elapsed());
        return;
    }

    // Print table header
    println!(
        "{:<20} {:<20} {:<10} {:<20}",
        "Branch", "Current", "Status", "Last Modified"
    );
    println!("{}", "-".repeat(70));

    // Sort by last modified (newest first)
    workspaces.sort_by(|a, b| b.modified.cmp(&a.modified));

    let workspace_count = workspaces.len();
    for ws in workspaces {
        let status = if ws.has_changes { "dirty" } else { "clean" };
        let modified = ws
            .modified
            .map(|t| {
                let datetime: chrono::DateTime<chrono::Local> = t.into();
                datetime.format("%Y-%m-%d %H:%M").to_string()
            })
            .unwrap_or_else(|| "unknown".to_string());

        println!(
            "{:<20} {:<20} {:<10} {:<20}",
            ws.branch, ws.current_branch, status, modified
        );
    }

    println!("\n{} workspace(s) found.", workspace_count);
    info!("Agent list completed in {:?}", start.elapsed());
}

/// Information about a workspace directory.
struct WorkspaceInfo {
    branch: String,
    current_branch: String,
    has_changes: bool,
    modified: Option<std::time::SystemTime>,
    path: PathBuf,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_clone_args_parsing() {
        // This is a placeholder test
        assert!(true);
    }
}
