//! Repository Manager
//!
//! Manages the lifecycle of Git bare repositories.
//! Provides operations for:
//! - Creating new bare repositories
//! - Installing post-receive hooks (embedded at compile time)
//! - Repository validation and error handling
//!
//! The post-receive hook is embedded in the binary and automatically installed
//! when a new repository is created, allowing CI jobs to be triggered on push.

use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

// Embed the post-receive hook script at compile time
const POST_RECEIVE_HOOK: &str = include_str!("../post-receive");

#[derive(Debug)]
#[allow(dead_code)]
pub enum RepoError {
    IoError(std::io::Error),
    GitError(String),
    AlreadyExists(String),
    InvalidPath(String),
}

impl std::fmt::Display for RepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoError::IoError(err) => write!(f, "IO error: {}", err),
            RepoError::GitError(msg) => write!(f, "Git error: {}", msg),
            RepoError::AlreadyExists(name) => write!(f, "Repository already exists: {}", name),
            RepoError::InvalidPath(msg) => write!(f, "Invalid path: {}", msg),
        }
    }
}

impl std::error::Error for RepoError {}

impl From<std::io::Error> for RepoError {
    fn from(err: std::io::Error) -> Self {
        RepoError::IoError(err)
    }
}

type Result<T> = std::result::Result<T, RepoError>;

#[derive(Clone)]
pub struct RepoManager {
    repos_base_path: PathBuf,
}

impl RepoManager {
    pub fn new<P: AsRef<Path>>(repos_base_path: P) -> Self {
        Self {
            repos_base_path: repos_base_path.as_ref().to_path_buf(),
        }
    }

    /// Create a new bare Git repository
    pub async fn create_bare_repo(&self, name: &str) -> Result<String> {
        let repo_path = self.repos_base_path.join(format!("{}.git", name));

        // Check if repo already exists
        if repo_path.exists() {
            return Err(RepoError::AlreadyExists(name.to_string()));
        }

        // Create base directory if it doesn't exist
        std::fs::create_dir_all(&self.repos_base_path)?;

        info!("Creating bare repository at: {}", repo_path.display());

        // Run git init --bare
        let output = Command::new("git")
            .args(&["init", "--bare", repo_path.to_str().unwrap()])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RepoError::GitError(format!(
                "Failed to create repository: {}",
                stderr
            )));
        }

        // Install post-receive hook
        self.install_post_receive_hook(&repo_path)?;

        Ok(repo_path.to_string_lossy().to_string())
    }

    /// Install post-receive hook from embedded script
    fn install_post_receive_hook(&self, repo_path: &Path) -> Result<()> {
        let hooks_dir = repo_path.join("hooks");
        let post_receive = hooks_dir.join("post-receive");

        info!(
            "Installing post-receive hook at: {}",
            post_receive.display()
        );

        // Write the embedded script to the hooks directory
        std::fs::write(&post_receive, POST_RECEIVE_HOOK)?;

        // Make it executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&post_receive)?.permissions();
            perms.set_mode(0o755); // rwxr-xr-x
            std::fs::set_permissions(&post_receive, perms)?;
        }

        info!("Post-receive hook installed");
        Ok(())
    }

    /// List all repositories in the base path
    pub async fn list_repos(&self) -> Result<Vec<(String, PathBuf)>> {
        let mut repos = Vec::new();

        let entries = std::fs::read_dir(&self.repos_base_path)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            // Check if it's a directory ending with .git
            if path.is_dir() && path.extension().and_then(|s| s.to_str()) == Some("git") {
                if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                    repos.push((name.to_string(), path));
                }
            }
        }

        Ok(repos)
    }

    /// Delete a repository (be careful!)
    #[allow(dead_code)]
    pub async fn delete_repo(&self, name: &str) -> Result<()> {
        let repo_path = self.repos_base_path.join(format!("{}.git", name));

        if !repo_path.exists() {
            return Err(RepoError::InvalidPath(format!(
                "Repository does not exist: {}",
                name
            )));
        }

        info!("Deleting repository at: {}", repo_path.display());
        std::fs::remove_dir_all(&repo_path)?;

        Ok(())
    }
}
