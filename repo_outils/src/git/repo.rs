//! Git2 operations: clone, pull, push, and submodule handling via git2.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::{error, info};

/// Errors that can occur during git operations.
#[derive(Debug)]
pub enum Git2Error {
    GitError(git2::Error),
    IoError(std::io::Error),
    SubmoduleError(String),
    AuthError(String),
    NotARepository,
    NoRemote,
}

impl std::fmt::Display for Git2Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Git2Error::GitError(e) => write!(f, "Git error: {}", e),
            Git2Error::IoError(e) => write!(f, "IO error: {}", e),
            Git2Error::SubmoduleError(msg) => write!(f, "Submodule error: {}", msg),
            Git2Error::AuthError(msg) => write!(f, "Auth error: {}", msg),
            Git2Error::NotARepository => write!(f, "Not a git repository"),
            Git2Error::NoRemote => write!(f, "No remote configured"),
        }
    }
}

impl std::error::Error for Git2Error {}

impl From<git2::Error> for Git2Error {
    fn from(err: git2::Error) -> Self {
        if err.code() == git2::ErrorCode::Auth {
            Git2Error::AuthError(err.to_string())
        } else {
            Git2Error::GitError(err)
        }
    }
}

impl From<std::io::Error> for Git2Error {
    fn from(err: std::io::Error) -> Self {
        Git2Error::IoError(err)
    }
}

type Result<T> = std::result::Result<T, Git2Error>;

/// Wraps git2::Repository with path tracking.
pub struct GitRepo {
    repo: git2::Repository,
    path: PathBuf,
}

impl std::fmt::Debug for GitRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitRepo").field("path", &self.path).finish()
    }
}

impl GitRepo {
    /// Open existing repository at path.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let repo = git2::Repository::open(&path).map_err(|e| {
            error!("Failed to open {}: {}", path.as_ref().display(), e);
            Git2Error::from(e)
        })?;

        Ok(Self {
            path: path.as_ref().to_path_buf(),
            repo,
        })
    }

    /// Clone repository from URL to local path using ssh-agent auth.
    pub fn clone<P: AsRef<Path>>(url: &str, path: P) -> Result<Self> {
        info!("Cloning {} to {}", url, path.as_ref().display());

        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(
            |_url: &str, username_from_url: Option<&str>, _allowed_types: git2::CredentialType| {
                git2::Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
            },
        );

        let mut fetch_options = git2::FetchOptions::new();
        fetch_options.remote_callbacks(callbacks);

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_options);

        let repo = builder.clone(url, path.as_ref()).map_err(|e| {
            error!("Failed to clone {}: {}", url, e);
            Git2Error::from(e)
        })?;

        info!("Clone completed");

        Ok(Self {
            path: path.as_ref().to_path_buf(),
            repo,
        })
    }

    /// Initialize all submodules in the repository.
    pub fn init_submodules(&self) -> Result<Vec<SubmoduleInfo>> {
        info!("Initializing submodules");

        let mut submodules = self.repo.submodules().map_err(|e| {
            error!("Failed to get submodules: {}", e);
            Git2Error::from(e)
        })?;

        let mut initialized = Vec::new();

        for submodule in submodules.iter_mut() {
            let name = submodule.name().unwrap_or("unknown").to_string();
            let path = submodule.path().to_path_buf();
            let url = submodule.url().unwrap_or("").to_string();

            info!("Initializing submodule: {} ({})", name, url);

            submodule.init(true).map_err(|e| {
                error!("Failed to init {}: {}", name, e);
                Git2Error::from(e)
            })?;

            initialized.push(SubmoduleInfo { name, path, url });
        }

        Ok(initialized)
    }

    /// List all submodules with their metadata.
    pub fn list_submodules(&self) -> Result<Vec<SubmoduleInfo>> {
        let submodules = self.repo.submodules().map_err(|e| {
            error!("Failed to list submodules: {}", e);
            Git2Error::from(e)
        })?;

        let mut result = Vec::new();

        for submodule in submodules.iter() {
            let name = submodule.name().unwrap_or("unknown").to_string();
            let path = submodule.path().to_path_buf();
            let url = submodule.url().unwrap_or("").to_string();

            result.push(SubmoduleInfo { name, path, url });
        }

        Ok(result)
    }

    /// Pull latest changes: fetch + merge with conflict detection.
    pub fn pull(&self) -> Result<()> {
        info!("Pulling latest changes");

        let mut remote = self.repo.find_remote("origin").map_err(|_| {
            error!("No origin remote found");
            Git2Error::NoRemote
        })?;

        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(
            |_url: &str, username_from_url: Option<&str>, _allowed_types: git2::CredentialType| {
                git2::Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
            },
        );

        let mut fetch_options = git2::FetchOptions::new();
        fetch_options.remote_callbacks(callbacks);

        remote
            .fetch(&["main", "master"], Some(&mut fetch_options), None)
            .map_err(|e| {
                error!("Fetch failed: {}", e);
                Git2Error::from(e)
            })?;

        let fetch_head = self.repo.find_reference("FETCH_HEAD")?;
        let fetch_commit = self.repo.reference_to_annotated_commit(&fetch_head)?;

        let head = self.repo.head()?;
        let head_commit = head.peel_to_commit()?;

        let mut merge_options = git2::MergeOptions::new();

        self.repo
            .merge(&[&fetch_commit], Some(&mut merge_options), None)
            .map_err(|e| {
                error!("Merge failed: {}", e);
                Git2Error::from(e)
            })?;

        if self.repo.index()?.has_conflicts() {
            error!("Merge conflicts detected");
            self.repo.cleanup_state()?;
            return Err(Git2Error::GitError(git2::Error::from_str(
                "Merge conflicts detected",
            )));
        }

        let signature = self.repo.signature()?;
        let message = "Merge remote-tracking branch";

        let mut index = self.repo.index()?;
        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;

        let fetch_commit_obj = self.repo.find_commit(fetch_commit.id())?;

        let merge_commit_oid = self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &[&head_commit, &fetch_commit_obj],
        )?;

        let obj = self.repo.find_object(merge_commit_oid, None)?;
        self.repo.reset(&obj, git2::ResetType::Hard, None)?;

        self.repo.cleanup_state()?;

        info!("Pull completed");
        Ok(())
    }

    /// Push local commits to remote using ssh-agent auth.
    pub fn push(&self) -> Result<()> {
        info!("Pushing to remote");

        let mut remote = self.repo.find_remote("origin").map_err(|_| {
            error!("No origin remote found");
            Git2Error::NoRemote
        })?;

        let head = self.repo.head()?;
        let branch_name = head.shorthand().unwrap_or("main");

        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(
            |_url: &str, username_from_url: Option<&str>, _allowed_types: git2::CredentialType| {
                git2::Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
            },
        );

        let mut push_options = git2::PushOptions::new();
        push_options.remote_callbacks(callbacks);

        let refspec = format!("refs/heads/{}:refs/heads/{}", branch_name, branch_name);

        remote
            .push(&[&refspec], Some(&mut push_options))
            .map_err(|e| {
                error!("Push failed: {}", e);
                Git2Error::from(e)
            })?;

        info!("Push completed");
        Ok(())
    }

    /// Return repository path (read-only access).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Submodule metadata for filtering operations.
#[derive(Debug, Clone)]
pub struct SubmoduleInfo {
    name: String,
    path: PathBuf,
    url: String,
}

impl SubmoduleInfo {
    /// Return submodule name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return submodule path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return submodule URL.
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// Filter submodules by include/exclude lists (exclude applied first).
pub fn filter_submodules(
    submodules: Vec<SubmoduleInfo>,
    include: &Option<Vec<String>>,
    exclude: &Option<Vec<String>>,
) -> Vec<SubmoduleInfo> {
    let include_set: Option<HashSet<&str>> = include
        .as_ref()
        .map(|v| v.iter().map(|s| s.as_str()).collect());
    let exclude_set: Option<HashSet<&str>> = exclude
        .as_ref()
        .map(|v| v.iter().map(|s| s.as_str()).collect());

    submodules
        .into_iter()
        .filter(|sub| {
            if let Some(exclude) = &exclude_set {
                if exclude.contains(sub.name.as_str()) {
                    return false;
                }
            }

            if let Some(include) = &include_set {
                return include.contains(sub.name.as_str());
            }

            true
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_submodules() {
        let submodules = vec![
            SubmoduleInfo {
                name: "docs".to_string(),
                path: PathBuf::from("docs"),
                url: "https://example.com/docs.git".to_string(),
            },
            SubmoduleInfo {
                name: "config".to_string(),
                path: PathBuf::from("config"),
                url: "https://example.com/config.git".to_string(),
            },
            SubmoduleInfo {
                name: "code".to_string(),
                path: PathBuf::from("code"),
                url: "https://example.com/code.git".to_string(),
            },
        ];

        // Test include filter
        let include = Some(vec!["docs".to_string(), "config".to_string()]);
        let filtered = filter_submodules(submodules.clone(), &include, &None);
        assert_eq!(filtered.len(), 2);
        assert!(
            filtered
                .iter()
                .all(|s| s.name == "docs" || s.name == "config")
        );

        // Test exclude filter
        let exclude = Some(vec!["docs".to_string()]);
        let filtered = filter_submodules(submodules.clone(), &None, &exclude);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|s| s.name != "docs"));

        // Test both
        let include = Some(vec![
            "docs".to_string(),
            "config".to_string(),
            "code".to_string(),
        ]);
        let exclude = Some(vec!["code".to_string()]);
        let filtered = filter_submodules(submodules.clone(), &include, &exclude);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|s| s.name != "code"));
    }
}
