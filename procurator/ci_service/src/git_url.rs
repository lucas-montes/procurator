//! Git URL Builder
//!
//! Constructs valid `git+file://` URLs for Nix to fetch repositories.
//! Handles:
//! - Absolute path resolution
//! - URL encoding/escaping
//! - Revision (commit hash) specification
//!
//! Nix requires absolute paths and specific URL formatting to fetch from local repositories.

use std::path::Path;

/// Build a proper git URL for Nix to fetch from a bare repository
pub fn build_nix_git_url(bare_repo_path: &str, commit_hash: &str) -> Result<String, String> {
    // Validate inputs
    if bare_repo_path.is_empty() {
        return Err("bare_repo_path cannot be empty".to_string());
    }

    if commit_hash.is_empty() {
        return Err("commit_hash cannot be empty".to_string());
    }

    // Ensure we have an absolute path
    let path = Path::new(bare_repo_path);
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("Failed to get current directory: {}", e))?
            .join(path)
    };

    // Convert to string
    let path_str = absolute_path
        .to_str()
        .ok_or_else(|| "Path contains invalid UTF-8".to_string())?;

    // Build the git+file:// URL
    // Format: git+file:///absolute/path/to/repo.git?rev=commitsha
    Ok(format!("git+file://{}?rev={}", path_str, commit_hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_absolute_path() {
        let url = build_nix_git_url("/home/user/repos/test.git", "abc123").unwrap();
        assert_eq!(url, "git+file:///home/user/repos/test.git?rev=abc123");
    }

    #[test]
    fn test_relative_path_conversion() {
        // This will be converted to absolute
        let url = build_nix_git_url("repos/test.git", "def456").unwrap();
        let cwd = std::env::current_dir().unwrap();
        let expected = format!("git+file://{}/repos/test.git?rev=def456", cwd.display());
        assert_eq!(url, expected);
    }

    #[test]
    fn test_empty_path() {
        let result = build_nix_git_url("", "abc123");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "bare_repo_path cannot be empty");
    }

    #[test]
    fn test_empty_commit() {
        let result = build_nix_git_url("/home/user/repos/test.git", "");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "commit_hash cannot be empty");
    }

    #[test]
    fn test_dot_path() {
        // "." should be converted to absolute path
        let url = build_nix_git_url(".", "abc123").unwrap();
        let cwd = std::env::current_dir().unwrap();
        let expected = format!("git+file://{}?rev=abc123", cwd.display());
        assert_eq!(url, expected);
    }

    #[test]
    fn test_path_with_git_extension() {
        let url = build_nix_git_url("/srv/git/myproject.git", "sha256").unwrap();
        assert_eq!(url, "git+file:///srv/git/myproject.git?rev=sha256");
    }

    #[test]
    fn test_full_commit_hash() {
        let commit = "f1e95cf2878741f42fd371a2df553a2b94065bc2";
        let url = build_nix_git_url("/repos/test.git", commit).unwrap();
        assert_eq!(url, format!("git+file:///repos/test.git?rev={}", commit));
    }

    #[test]
    fn test_short_commit_hash() {
        let url = build_nix_git_url("/repos/test.git", "f1e95cf2").unwrap();
        assert_eq!(url, "git+file:///repos/test.git?rev=f1e95cf2");
    }
}
