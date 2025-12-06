//! Repository Manager
//!
//! Manages the lifecycle of Git bare repositories.
//! Provides operations for:
//! - Creating new bare repositories
//! - Repository validation and error handling
//!
//! The post-receive hook is embedded in the binary and automatically installed
//! when a new repository is created, allowing CI jobs to be triggered on push.

use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

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

#[derive(Debug)]
pub struct RepoPath {
    base_path: PathBuf,
    username: String,
    repo_name: String,
}

impl RepoPath {
    /// Create a new RepoPath from components
    pub fn new(
        base_path: impl AsRef<Path>,
        username: impl Into<String>,
        repo_name: impl Into<String>,
    ) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
            username: username.into(),
            repo_name: repo_name.into(),
        }
    }

    /// Get the repository name (without .git extension)
    pub fn repo_name(&self) -> &str {
        &self.repo_name
    }

    /// Get the full filesystem path to the bare repository
    /// Returns: base_path/username/repo.git
    pub fn bare_repo_path(&self) -> PathBuf {
        self.base_path
            .join(&self.username)
            .join(format!("{}.git", self.repo_name))
    }

    /// Build full SSH clone URL with domain
    pub fn to_ssh_url(&self, domain: &str) -> String {
        format!("git@{}:{}/{}.git", domain, self.username, self.repo_name)
    }

    /// Get the path as a string
    pub fn to_path_string(&self) -> String {
        self.bare_repo_path().to_string_lossy().to_string()
    }

    /// Build a Nix-compatible git+file:// URL (without revision)
    pub fn to_nix_url(&self) -> String {
        format!("git+file://{}", self.bare_repo_path().display())
    }

    /// Build a Nix-compatible git+file:// URL with a specific revision
    pub fn to_nix_url_with_rev(&self, commit_hash: &str) -> String {
        format!(
            "git+file://{}?rev={}",
            self.bare_repo_path().display(),
            commit_hash
        )
    }
}

impl std::fmt::Display for RepoPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{}/{}.git",
            self.base_path.display(),
            self.username,
            self.repo_name
        )
    }
}

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

    /// Create a RepoPath for the given username and repo
    pub fn repo_path(&self, username: &str, repo: &str) -> RepoPath {
        RepoPath::new(&self.repos_base_path, username, repo)
    }

    /// Create a new bare Git repository
    pub async fn create_bare_repo(&self, username: &str, repo: &str) -> Result<RepoPath> {
        let repo_path = self.repo_path(username, repo);
        let bare_path = repo_path.bare_repo_path();

        // Check if repo already exists
        if bare_path.exists() {
            return Err(RepoError::AlreadyExists(repo.to_string()));
        }

        info!(repo_path=%repo_path, "Creating bare repository at");

        let output = Command::new("git")
            .args(["init", "--bare", "--shared=group"])
            .arg(&bare_path)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RepoError::GitError(format!(
                "Failed to create repository: {}",
                stderr
            )));
        }

        Ok(repo_path)
    }

    /// List all repositories for a user
    pub async fn list_repos(&self, username: &str) -> Result<Vec<RepoPath>> {
        let mut repos = Vec::new();
        let user_dir = self.repos_base_path.join(username);

        let entries = std::fs::read_dir(&user_dir)?;

        info!(user_dir=?user_dir, "Listing repositories");

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            // Check if it's a directory ending with .git
            if path.is_dir() && path.extension().and_then(|s| s.to_str()) == Some("git") {
                if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                    repos.push(self.repo_path(username, name));
                }
            }
        }

        info!(count = repos.len(), "Repositories found");

        Ok(repos)
    }

    /// Delete a repository (be careful!)
    #[allow(dead_code)]
    pub async fn delete_repo(&self, username: &str, repo: &str) -> Result<()> {
        let repo_path = self.repo_path(username, repo);
        let bare_path = repo_path.bare_repo_path();

        if !bare_path.exists() {
            return Err(RepoError::InvalidPath(format!(
                "Repository does not exist: {}",
                repo_path
            )));
        }

        info!("Deleting repository at: {}", repo_path);
        std::fs::remove_dir_all(&bare_path)?;

        Ok(())
    }
}
