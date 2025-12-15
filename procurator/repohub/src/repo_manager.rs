//! Repository Manager
//!
//! Orchestration layer that combines database operations with Git operations.
//! Provides high-level repository management functionality.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;
use sqlx::FromRow;

use crate::database::{Database, DatabaseError};
use crate::git_manager::{self, RepoPath, RepoError};

type Result<T> = std::result::Result<T, DatabaseError>;

// ============================================================================
// Simple DTOs for repository management
// ============================================================================

#[derive(Debug, Clone, FromRow)]
struct User {
    id: i64,
    username: String,
}

#[derive(Debug, Clone, FromRow)]
struct Repo {
    id: i64,
    name: String,
    path: String,
}

// ============================================================================
// Repository Store - CRUD operations for repos and users
// ============================================================================

#[derive(Clone)]
pub struct RepositoryStore {
    db: Database,
}

impl RepositoryStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    // ========================================================================
    // User Operations
    // ========================================================================

    pub async fn create_user(&self, username: &str, email: Option<&str>) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO users (username, email) VALUES (?, ?)"
        )
        .bind(username)
        .bind(email)
        .execute(self.db.pool())
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn get_user_by_username(&self, username: &str) -> Result<Option<User>> {
        let user = sqlx::query_as::<_, User>(
            "SELECT id, username FROM users WHERE username = ?"
        )
        .bind(username)
        .fetch_optional(self.db.pool())
        .await?;

        Ok(user)
    }

    pub async fn get_or_create_user(&self, username: &str) -> Result<i64> {
        if let Some(user) = self.get_user_by_username(username).await? {
            return Ok(user.id);
        }

        self.create_user(username, None).await
    }

    pub async fn get_all_users(&self) -> Result<Vec<(String, i64)>> {
        let users = sqlx::query_as::<_, (String, i64)>(
            "SELECT username, id FROM users ORDER BY username"
        )
        .fetch_all(self.db.pool())
        .await?;

        Ok(users)
    }

    pub async fn get_all_users_with_repo_count(&self) -> Result<Vec<(String, i64)>> {
        let users = sqlx::query_as::<_, (String, i64)>(
            r#"
            SELECT u.username, COUNT(r.id) as repo_count
            FROM users u
            LEFT JOIN repos r ON u.id = r.user_id
            GROUP BY u.id, u.username
            ORDER BY u.username
            "#
        )
        .fetch_all(self.db.pool())
        .await?;

        Ok(users)
    }

    // ========================================================================
    // Repository Operations
    // ========================================================================

    pub async fn create_repo(&self, user_id: i64, name: &str, path: &str, description: Option<&str>) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO repos (user_id, name, path, description) VALUES (?, ?, ?, ?)"
        )
        .bind(user_id)
        .bind(name)
        .bind(path)
        .bind(description)
        .execute(self.db.pool())
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn get_repo_by_path(&self, path: &str) -> Result<Option<Repo>> {
        let repo = sqlx::query_as::<_, Repo>(
            "SELECT id, name, path FROM repos WHERE path = ?"
        )
        .bind(path)
        .fetch_optional(self.db.pool())
        .await?;

        Ok(repo)
    }

    pub async fn get_or_create_repo(&self, repo_path: &str) -> Result<i64> {
        // Try to get existing repo
        if let Some(repo) = self.get_repo_by_path(repo_path).await? {
            return Ok(repo.id);
        }

        // Extract repo name and username from path
        let path_buf = PathBuf::from(repo_path);
        let repo_name = path_buf
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| DatabaseError::InvalidData("Invalid repo path".to_string()))?;

        let username = path_buf
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .ok_or_else(|| DatabaseError::InvalidData("Invalid repo path".to_string()))?;

        // Get or create user
        let user_id = self.get_or_create_user(username).await?;

        // Create repo
        self.create_repo(user_id, repo_name, repo_path, None).await
    }

    pub async fn get_all_repos(&self) -> Result<Vec<Repo>> {
        let repos = sqlx::query_as::<_, Repo>(
            "SELECT id, name, path FROM repos ORDER BY id DESC"
        )
        .fetch_all(self.db.pool())
        .await?;

        Ok(repos)
    }

    pub async fn get_repos_for_user(&self, username: &str) -> Result<Vec<Repo>> {
        let repos = sqlx::query_as::<_, Repo>(
            r#"
            SELECT r.id, r.name, r.path
            FROM repos r
            JOIN users u ON r.user_id = u.id
            WHERE u.username = ?
            ORDER BY r.id DESC
            "#
        )
        .bind(username)
        .fetch_all(self.db.pool())
        .await?;

        Ok(repos)
    }

    pub async fn list_repos_with_build_counts(&self) -> Result<Vec<(String, String, i64)>> {
        let repos = sqlx::query_as::<_, (String, String, i64)>(
            r#"
            SELECT
                r.name,
                r.path,
                COUNT(b.id) as builds_count
            FROM repos r
            LEFT JOIN builds b ON r.id = b.repo_id
            GROUP BY r.id, r.name, r.path
            ORDER BY r.created_at DESC
            "#,
        )
        .fetch_all(self.db.pool())
        .await?;

        Ok(repos)
    }
}

// ============================================================================
// Repository Manager - High-level repository operations
// ============================================================================

#[derive(Clone)]
pub struct RepoManager {
    repos_base_path: PathBuf,
    repo_store: Option<Arc<RepositoryStore>>,
}

impl RepoManager {
    pub fn new<P: AsRef<Path>>(repos_base_path: P) -> Self {
        Self {
            repos_base_path: repos_base_path.as_ref().to_path_buf(),
            repo_store: None,
        }
    }

    /// Set the repository store reference for database operations
    pub fn with_repo_store(mut self, repo_store: Arc<RepositoryStore>) -> Self {
        self.repo_store = Some(repo_store);
        self
    }

    /// Create a RepoPath for the given username and repo
    pub fn repo_path(&self, username: &str, repo: &str) -> RepoPath {
        RepoPath::new(&self.repos_base_path, username, repo)
    }

    /// Create a new bare Git repository (combines git operation with database record)
    pub async fn create_bare_repo(&self, username: &str, repo: &str) -> std::result::Result<RepoPath, RepoError> {
        let repo_path = self.repo_path(username, repo);
        let bare_path = repo_path.bare_repo_path();

        // Create the git repository
        git_manager::create_bare_repo(&bare_path)?;

        // Register in database if repo_store is available
        if let Some(repo_store) = &self.repo_store {
            let path_str = repo_path.to_string();
            repo_store.get_or_create_repo(&path_str)
                .await
                .map_err(|e| RepoError::GitError(format!("Failed to register repo in database: {}", e)))?;
        }

        Ok(repo_path)
    }

    /// List all repositories for a user from the database
    pub async fn list_repos(&self, username: &str) -> std::result::Result<Vec<RepoPath>, RepoError> {
        if let Some(repo_store) = &self.repo_store {
            // Use database to get repos
            let db_repos = repo_store.get_repos_for_user(username)
                .await
                .map_err(|e| RepoError::GitError(format!("Failed to query database: {}", e)))?;

            let repos: Vec<RepoPath> = db_repos
                .into_iter()
                .filter_map(|repo| {
                    // Parse the path to extract username and repo name
                    let path_buf = PathBuf::from(&repo.path);
                    let repo_name = path_buf.file_stem()?.to_str()?;
                    let username = path_buf.parent()?.file_name()?.to_str()?;
                    Some(self.repo_path(username, repo_name))
                })
                .collect();

            info!(count = repos.len(), username = username, "Repositories found from database");
            Ok(repos)
        } else {
            // Fallback to filesystem scanning if repo_store is not set
            self.list_repos_from_fs(username).await
        }
    }

    /// List all repositories for a user from filesystem (legacy/fallback)
    async fn list_repos_from_fs(&self, username: &str) -> std::result::Result<Vec<RepoPath>, RepoError> {
        let mut repos = Vec::new();
        let user_dir = self.repos_base_path.join(username);

        let entries = std::fs::read_dir(&user_dir)?;

        info!(user_dir=?user_dir, "Listing repositories from filesystem");

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

        info!(count = repos.len(), "Repositories found from filesystem");

        Ok(repos)
    }

    /// Delete a repository (be careful! - deletes both git repo and database record)
    #[allow(dead_code)]
    pub async fn delete_repo(&self, username: &str, repo: &str) -> std::result::Result<(), RepoError> {
        let repo_path = self.repo_path(username, repo);
        let bare_path = repo_path.bare_repo_path();

        // Delete from git
        git_manager::delete_repo(&bare_path)?;

        // TODO: Also delete from database if repo_store is available
        // This requires adding a delete method to RepositoryStore

        Ok(())
    }
}
