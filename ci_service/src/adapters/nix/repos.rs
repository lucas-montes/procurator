//! Where a repository lives on disk.

use std::path::{Path, PathBuf};

use crate::domain::run::RepoName;

/// The bare repository of `repo` under `base`: `<base>/<owner>/<name>.git`.
/// Why: the path comes from a validated name only, never from a request.
///
/// # Errors
///
/// When it is not a directory.
pub fn bare_repo(base: &Path, repo: &RepoName) -> Result<PathBuf, String> {
    let path = base.join(repo.owner()).join(format!("{}.git", repo.name()));
    if path.is_dir() {
        Ok(path)
    } else {
        Err(format!(
            "repository {} not found at {}",
            repo.as_str(),
            path.display()
        ))
    }
}
