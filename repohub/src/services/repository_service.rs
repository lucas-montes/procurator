use std::path::PathBuf;
use std::fmt;
use repo_outils::git::{RepoPath, create_bare_repo, clone_into_bare};
use repo_outils::nix::{FlakeMetadata, Infrastructure};
use crate::config::Config;

#[derive(Debug)]
pub enum RepositoryError {
    DirectoryCreation(std::io::Error),
    BareRepoCreation(String),
    CloneFailed(String),
    InvalidPath,
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RepositoryError::DirectoryCreation(e) => write!(f, "Failed to create directory: {}", e),
            RepositoryError::BareRepoCreation(msg) => write!(f, "Failed to create bare repository: {}", msg),
            RepositoryError::CloneFailed(msg) => write!(f, "Failed to clone repository: {}", msg),
            RepositoryError::InvalidPath => write!(f, "Invalid repository path"),
        }
    }
}

impl std::error::Error for RepositoryError {}

impl From<std::io::Error> for RepositoryError {
    fn from(err: std::io::Error) -> Self {
        RepositoryError::DirectoryCreation(err)
    }
}

#[derive(Clone)]
pub struct RepositoryService {
    repos_base_path: PathBuf,
    domain: String,
}

impl RepositoryService {
    pub fn new(config: &Config) -> Self {
        Self {
            repos_base_path: PathBuf::from(&config.repos_base_path),
            domain: config.domain.clone(),
        }
    }

    /// Create a new bare repository or clone from a remote URL
    pub fn create_or_clone_repository(
        &self,
        username: &str,
        repo_name: &str,
        git_url: Option<&str>,
    ) -> Result<String, RepositoryError> {
        let repo_path = RepoPath::new(&self.repos_base_path, username, repo_name);
        let bare_path = repo_path.bare_repo_path();

        let git_url_to_store = if let Some(remote_url) = git_url {
            // Clone from remote
            clone_into_bare(&bare_path, remote_url)
                .map_err(|e| RepositoryError::CloneFailed(e.to_string()))?;

            tracing::info!(
                remote = remote_url,
                path = %bare_path.display(),
                "Cloned remote into bare repo"
            );

            remote_url.to_string()
        } else {
            // Create new bare repository
            std::fs::create_dir_all(bare_path.parent().ok_or(RepositoryError::InvalidPath)?)
                .map_err(RepositoryError::DirectoryCreation)?;

            create_bare_repo(&bare_path)
                .map_err(|e| RepositoryError::BareRepoCreation(e.to_string()))?;

            tracing::info!(path = %bare_path.display(), "Created bare repository");

            repo_path.to_ssh_url(&self.domain)
        };

        Ok(git_url_to_store)
    }

    /// Parse flake metadata for a repository (best-effort)
    pub fn parse_flake_metadata(
        &self,
        username: &str,
        repo_name: &str,
    ) -> Option<FlakeMetadata> {
        let repo_path = RepoPath::new(&self.repos_base_path, username, repo_name);

        match FlakeMetadata::try_from(&repo_path) {
            Ok(metadata) => {
                tracing::info!(?metadata, "Found flake metadata for repo");
                Some(metadata)
            }
            Err(e) => {
                tracing::debug!(error = ?e, "No flake metadata or failed to parse");
                None
            }
        }
    }

    /// Parse infrastructure specification from a repository (best-effort)
    pub fn parse_infrastructure(
        &self,
        username: &str,
        repo_name: &str,
    ) -> Option<Infrastructure> {
        let repo_path = RepoPath::new(&self.repos_base_path, username, repo_name);

        match Infrastructure::try_from(&repo_path) {
            Ok(infra) => {
                tracing::info!("Parsed infrastructure from flake");
                Some(infra)
            }
            Err(e) => {
                tracing::debug!(error = ?e, "No infrastructure found in flake");
                None
            }
        }
    }

    /// Get the repository path for a given user and repo
    pub fn get_repo_path(&self, username: &str, repo_name: &str) -> RepoPath {
        RepoPath::new(&self.repos_base_path, username, repo_name)
    }
}
