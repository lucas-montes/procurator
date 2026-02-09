//! Git Manager
//!
//! Provides Git-specific operations and utilities.
//! Handles:
//! - Git command execution
//! - Repository path management
//! - URL generation for Git operations

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
        let path = self.base_path.join(&self.username).join(&self.repo_name);
        // Manually add .git extension to handle names with dots correctly
        let mut os_string = path.into_os_string();
        os_string.push(".git");
        os_string.into()
    }

    /// Build full SSH clone URL with domain
    pub fn to_ssh_url(&self, domain: &str) -> String {
        format!("git@{}:{}/{}.git", domain, self.username, self.repo_name)
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

// ============================================================================
// Git Operations
// ============================================================================

/// Create a new bare Git repository at the specified path
pub fn create_bare_repo(bare_path: &Path) -> Result<()> {
    if bare_path.exists() {
        return Err(RepoError::AlreadyExists(bare_path.display().to_string()));
    }

    info!("Creating bare repository at: {}", bare_path.display());

    let output = Command::new("git")
        .args(["init", "--bare", "--shared=group"])
        .arg(bare_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RepoError::GitError(format!(
            "Failed to create repository: {}",
            stderr
        )));
    }

    Ok(())
}

/// Clone a remote repository into a bare repository at the specified path
pub fn clone_into_bare(bare_path: &Path, remote_url: &str) -> Result<()> {
    if bare_path.exists() {
        return Err(RepoError::AlreadyExists(bare_path.display().to_string()));
    }

    info!("Cloning remote '{}' into bare repository at: {}", remote_url, bare_path.display());

    let output = Command::new("git")
        .args(["clone", "--bare", remote_url, &bare_path.to_string_lossy()])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RepoError::GitError(format!("Failed to clone repository: {}", stderr)));
    }

    Ok(())
}

/// Delete a Git repository (be careful!)
pub fn delete_repo(bare_path: &Path) -> Result<()> {
    if !bare_path.exists() {
        return Err(RepoError::InvalidPath(format!(
            "Repository does not exist: {}",
            bare_path.display()
        )));
    }

    info!("Deleting repository at: {}", bare_path.display());
    std::fs::remove_dir_all(bare_path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_path_new() {
        let path = RepoPath::new("/base/path", "testuser", "myrepo");
        assert_eq!(path.repo_name(), "myrepo");
    }


    #[test]
    fn test_repo_path_bare_repo_path_various_names() {
        // Simple name
        let path = RepoPath::new("/base", "user", "repo");
        assert_eq!(path.bare_repo_path(), PathBuf::from("/base/user/repo.git"));

        // Name with hyphens
        let path = RepoPath::new("/base", "user", "my-repo");
        assert_eq!(path.bare_repo_path(), PathBuf::from("/base/user/my-repo.git"));

        // Name with underscores
        let path = RepoPath::new("/base", "user", "my_repo");
        assert_eq!(path.bare_repo_path(), PathBuf::from("/base/user/my_repo.git"));

        // Name with numbers
        let path = RepoPath::new("/base", "user", "repo123");
        assert_eq!(path.bare_repo_path(), PathBuf::from("/base/user/repo123.git"));
    }

    #[test]
    fn test_repo_path_to_ssh_url() {
        let path = RepoPath::new("/base/path", "testuser", "myrepo");
        let ssh_url = path.to_ssh_url("example.com");
        assert_eq!(ssh_url, "git@example.com:testuser/myrepo.git");
    }

    #[test]
    fn test_repo_path_to_ssh_url_various_domains() {
        let path = RepoPath::new("/base", "user", "repo");

        assert_eq!(path.to_ssh_url("github.com"), "git@github.com:user/repo.git");
        assert_eq!(path.to_ssh_url("gitlab.com"), "git@gitlab.com:user/repo.git");
        assert_eq!(path.to_ssh_url("localhost"), "git@localhost:user/repo.git");
        assert_eq!(path.to_ssh_url("192.168.1.1"), "git@192.168.1.1:user/repo.git");
    }

    #[test]
    fn test_repo_path_to_nix_url() {
        let path = RepoPath::new("/base/path", "testuser", "myrepo");
        let nix_url = path.to_nix_url();
        assert_eq!(nix_url, "git+file:///base/path/testuser/myrepo.git");
    }


    #[test]
    fn test_repo_path_to_nix_url_with_various_commits() {
        let path = RepoPath::new("/base", "user", "repo");

        // Short commit hash
        let url = path.to_nix_url_with_rev("abc123");
        assert_eq!(url, "git+file:///base/user/repo.git?rev=abc123");

        // Full commit hash
        let url = path.to_nix_url_with_rev("abc123def456789012345678901234567890abcd");
        assert_eq!(url, "git+file:///base/user/repo.git?rev=abc123def456789012345678901234567890abcd");
    }


    #[test]
    fn test_repo_path_display() {
        let path = RepoPath::new("/base/path", "testuser", "myrepo");
        let display = format!("{}", path);
        assert_eq!(display, "/base/path/testuser/myrepo.git");
    }

    #[test]
    fn test_repo_path_with_special_characters_in_username() {
        let path = RepoPath::new("/base", "user-name", "repo");
        assert_eq!(path.bare_repo_path(), PathBuf::from("/base/user-name/repo.git"));

        let path = RepoPath::new("/base", "user_name", "repo");
        assert_eq!(path.bare_repo_path(), PathBuf::from("/base/user_name/repo.git"));

        let path = RepoPath::new("/base", "user.name", "repo");
        assert_eq!(path.bare_repo_path(), PathBuf::from("/base/user.name/repo.git"));
    }

    #[test]
    fn test_repo_path_complex_paths() {
        // Test with path containing dots
        let path = RepoPath::new("/base", "user", "my.repo.name");
        assert_eq!(path.bare_repo_path(), PathBuf::from("/base/user/my.repo.name.git"));

        // Test with path containing numbers and special chars
        let path = RepoPath::new("/base", "user123", "repo-v2.0");
        assert_eq!(path.bare_repo_path(), PathBuf::from("/base/user123/repo-v2.0.git"));
    }

    #[test]
    fn test_nix_url_with_different_commit_formats() {
        let path = RepoPath::new("/base", "user", "repo");

        // Test with branch name-like rev
        let url = path.to_nix_url_with_rev("main");
        assert!(url.contains("?rev=main"));

        // Test with tag-like rev
        let url = path.to_nix_url_with_rev("v1.0.0");
        assert!(url.contains("?rev=v1.0.0"));
    }


    #[test]
    fn test_repo_path_all_url_methods() {
        let path = RepoPath::new("/base", "user", "repo");

        // All URL methods should produce valid URLs
        let ssh = path.to_ssh_url("example.com");
        let nix = path.to_nix_url();
        let nix_rev = path.to_nix_url_with_rev("abc123");

        assert!(ssh.contains("git@"));
        assert!(nix.starts_with("git+file://"));
        assert!(nix_rev.starts_with("git+file://"));
        assert!(nix_rev.contains("?rev="));
    }
}
