//! Database Infrastructure Layer
//!
//! Handles database connection, schema initialization, and provides
//! data access methods for users, projects, and repositories.

use std::{ops::Deref, str::FromStr};

use crate::domain::signals::{
    NormalizedCommit, NormalizedDeployment, NormalizedIssue, NormalizedPullRequest,
    NormalizedReview,
};
use serde::Serialize;
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use tracing::info;

#[derive(Debug)]
pub enum DatabaseError {
    Connection(sqlx::Error),
    Query(sqlx::Error),
    InvalidData(String),
    NotFound(String),
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseError::Connection(err) => write!(f, "Database connection error: {}", err),
            DatabaseError::Query(err) => write!(f, "Database query error: {}", err),
            DatabaseError::InvalidData(msg) => write!(f, "Invalid data: {}", msg),
            DatabaseError::NotFound(msg) => write!(f, "Not found: {}", msg),
        }
    }
}

impl std::error::Error for DatabaseError {}

impl From<sqlx::Error> for DatabaseError {
    fn from(err: sqlx::Error) -> Self {
        DatabaseError::Query(err)
    }
}

pub type Result<T> = std::result::Result<T, DatabaseError>;

/// Database row for users table
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub email: Option<String>,
    pub github_token: Option<String>,
    pub github_login: Option<String>,
    pub created_at: String,
}

/// Database row for projects table
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProjectRow {
    pub id: i64,
    pub name: String,
    pub owner_id: i64,
    pub description: Option<String>,
    pub created_at: String,
}

/// Database row for repositories table
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RepositoryRow {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub git_url: String,
    pub created_at: String,
}

/// Database row for project members table
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProjectMemberRow {
    pub project_id: i64,
    pub user_id: i64,
    pub role: String,
    pub created_at: String,
}

/// Database row for GitHub pull requests table
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GithubPullRequestRow {
    pub id: i64,
    pub github_id: i64,
    pub repository_id: i64,
    pub number: i32,
    pub title: String,
    pub author_id: Option<i64>,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub merged_at: Option<String>,
    pub additions: i32,
    pub deletions: i32,
    pub changed_files: i32,
}

/// Database row for GitHub reviews table
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GithubReviewRow {
    pub id: i64,
    pub github_id: i64,
    pub pull_request_id: i64,
    pub user_id: Option<i64>,
    pub state: String,
    pub submitted_at: String,
}

/// Database row for GitHub commits table
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GithubCommitRow {
    pub id: i64,
    pub github_sha: String,
    pub repository_id: i64,
    pub author_name: Option<String>,
    pub author_email: Option<String>,
    pub committer_name: Option<String>,
    pub committer_email: Option<String>,
    pub message: String,
    pub timestamp: String,
}

/// Database row for GitHub deployments table
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GithubDeploymentRow {
    pub id: i64,
    pub github_id: i64,
    pub repository_id: i64,
    pub sha: String,
    #[sqlx(rename = "ref")]
    pub ref_field: String,
    pub task: String,
    pub payload: Option<String>,
    pub environment: String,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
    pub creator_id: Option<i64>,
}

/// Database row for GitHub issues table
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GithubIssueRow {
    pub id: i64,
    pub github_id: i64,
    pub repository_id: i64,
    pub number: i32,
    pub title: String,
    pub author_id: Option<i64>,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
}

/// Database row for GitHub issue labels table
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GithubIssueLabelRow {
    pub id: i64,
    pub issue_id: i64,
    pub label_id: i64,
    pub label_name: String,
    pub label_color: String,
}

/// Database row for normalized signal persistence.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct NormalizedSignalRow {
    pub id: i64,
    pub repository_id: i64,
    pub signal_type: String,
    pub source_key: String,
    pub occurred_at: String,
    pub payload_json: String,
    pub ingested_at: String,
    pub updated_at: String,
}

/// Database row for persisted weekly metric snapshots.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct WeeklyMetricSnapshotRow {
    pub id: i64,
    pub repository_id: i64,
    pub week_start_utc: String,
    pub metric_version: String,
    pub metrics_json: String,
    pub window_days: i32,
    pub computed_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReviewChangeRow {
    pub id: i64,
    pub repository_id: i64,
    pub change_key: String,
    pub target_branch: String,
    pub subject: String,
    pub owner_user_id: i64,
    pub status: String,
    pub current_patch_set: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReviewPatchSetRow {
    pub change_id: i64,
    pub number: i32,
    pub revision: String,
    pub kind: String,
    pub uploader_user_id: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReviewApprovalRow {
    pub change_id: i64,
    pub user_id: i64,
    pub label: String,
    pub value: i32,
    pub updated_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReviewPolicyOverrideRow {
    pub scope_type: String,
    pub scope_id: i64,
    pub policy_json: String,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Deref for Database {
    type Target = SqlitePool;
    fn deref(&self) -> &Self::Target {
        &self.pool
    }
}

impl Database {
    pub async fn new(database_url: &str) -> Result<Self> {
        let database_config = SqliteConnectOptions::from_str(database_url)
            .map_err(|e| DatabaseError::Connection(e))?
            .create_if_missing(true);

        let pool = SqlitePool::connect_lazy_with(database_config);

        let db = Self { pool };
        db.initialize_tables().await?;

        info!("Database initialized at {}", database_url);
        Ok(db)
    }

    async fn initialize_tables(&self) -> Result<()> {
        // Users table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL UNIQUE,
                email TEXT,
                github_token TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Migration: add github_login column if missing (existing databases)
        let _ = sqlx::query(r#"ALTER TABLE users ADD COLUMN github_login TEXT"#)
            .execute(&self.pool)
            .await;

        // Projects table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS projects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                owner_id INTEGER NOT NULL,
                description TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (owner_id) REFERENCES users(id),
                UNIQUE(owner_id, name)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Repositories table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS repositories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                git_url TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (project_id) REFERENCES projects(id),
                UNIQUE(project_id, name)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Project members table (for collaboration)
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS project_members (
                project_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                role TEXT NOT NULL DEFAULT 'member',
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (project_id, user_id),
                FOREIGN KEY (project_id) REFERENCES projects(id),
                FOREIGN KEY (user_id) REFERENCES users(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Create indexes for performance
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_projects_owner_id ON projects(owner_id)")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_repositories_project_id ON repositories(project_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_project_members_user_id ON project_members(user_id)",
        )
        .execute(&self.pool)
        .await?;

        // Create tables for GitHub data storage
        sqlx::query(
            r#"
             CREATE TABLE IF NOT EXISTS github_pull_requests (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 github_id INTEGER NOT NULL UNIQUE,
                 repository_id INTEGER NOT NULL,
                 number INTEGER NOT NULL,
                 title TEXT NOT NULL,
                 author_id INTEGER,
                 state TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 closed_at TEXT,
                 merged_at TEXT,
                 additions INTEGER DEFAULT 0,
                 deletions INTEGER DEFAULT 0,
                 changed_files INTEGER DEFAULT 0,
                 FOREIGN KEY (repository_id) REFERENCES repositories(id),
                 FOREIGN KEY (author_id) REFERENCES users(id)
             )
             "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
             CREATE TABLE IF NOT EXISTS github_reviews (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 github_id INTEGER NOT NULL UNIQUE,
                 pull_request_id INTEGER NOT NULL,
                 user_id INTEGER,
                 state TEXT NOT NULL,
                 submitted_at TEXT NOT NULL,
                 FOREIGN KEY (pull_request_id) REFERENCES github_pull_requests(id),
                 FOREIGN KEY (user_id) REFERENCES users(id)
             )
             "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
             CREATE TABLE IF NOT EXISTS github_commits (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 github_sha TEXT NOT NULL UNIQUE,
                 repository_id INTEGER NOT NULL,
                 author_name TEXT,
                 author_email TEXT,
                 committer_name TEXT,
                 committer_email TEXT,
                 message TEXT NOT NULL,
                 timestamp TEXT NOT NULL,
                 FOREIGN KEY (repository_id) REFERENCES repositories(id)
             )
             "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
             CREATE TABLE IF NOT EXISTS github_deployments (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 github_id INTEGER NOT NULL UNIQUE,
                 repository_id INTEGER NOT NULL,
                 sha TEXT NOT NULL,
                 ref TEXT NOT NULL,
                 task TEXT NOT NULL,
                 payload TEXT,
                 environment TEXT NOT NULL,
                 state TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 creator_id INTEGER,
                 FOREIGN KEY (repository_id) REFERENCES repositories(id),
                 FOREIGN KEY (creator_id) REFERENCES users(id)
             )
             "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
             CREATE TABLE IF NOT EXISTS github_issues (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 github_id INTEGER NOT NULL UNIQUE,
                 repository_id INTEGER NOT NULL,
                 number INTEGER NOT NULL,
                 title TEXT NOT NULL,
                 author_id INTEGER,
                 state TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 closed_at TEXT,
                 FOREIGN KEY (repository_id) REFERENCES repositories(id),
                 FOREIGN KEY (author_id) REFERENCES users(id)
             )
             "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
             CREATE TABLE IF NOT EXISTS github_issue_labels (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 issue_id INTEGER NOT NULL,
                 label_id INTEGER NOT NULL,
                 label_name TEXT NOT NULL,
                 label_color TEXT NOT NULL,
                 FOREIGN KEY (issue_id) REFERENCES github_issues(id)
             )
             "#,
        )
        .execute(&self.pool)
        .await?;

        // Create indexes for GitHub tables for better query performance
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_github_pull_requests_repo_id ON github_pull_requests(repository_id)")
             .execute(&self.pool)
             .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_github_pull_requests_number ON github_pull_requests(number)")
             .execute(&self.pool)
             .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_github_reviews_pr_id ON github_reviews(pull_request_id)")
             .execute(&self.pool)
             .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_github_commits_repo_id ON github_commits(repository_id)")
             .execute(&self.pool)
             .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_github_deployments_repo_id ON github_deployments(repository_id)")
             .execute(&self.pool)
             .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_github_issues_repo_id ON github_issues(repository_id)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_github_issue_labels_issue_id ON github_issue_labels(issue_id)")
             .execute(&self.pool)
             .await?;

        // Gerrit-like review changes
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS review_changes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repository_id INTEGER NOT NULL,
                change_key TEXT NOT NULL UNIQUE,
                target_branch TEXT NOT NULL,
                subject TEXT NOT NULL,
                owner_user_id INTEGER NOT NULL,
                status TEXT NOT NULL,
                current_patch_set INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (repository_id) REFERENCES repositories(id),
                FOREIGN KEY (owner_user_id) REFERENCES users(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS review_patch_sets (
                change_id INTEGER NOT NULL,
                number INTEGER NOT NULL,
                revision TEXT NOT NULL,
                kind TEXT NOT NULL,
                uploader_user_id INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (change_id, number),
                FOREIGN KEY (change_id) REFERENCES review_changes(id),
                FOREIGN KEY (uploader_user_id) REFERENCES users(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS review_approvals (
                change_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                label TEXT NOT NULL,
                value INTEGER NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (change_id, user_id, label),
                FOREIGN KEY (change_id) REFERENCES review_changes(id),
                FOREIGN KEY (user_id) REFERENCES users(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS review_policy_overrides (
                scope_type TEXT NOT NULL,
                scope_id INTEGER NOT NULL,
                policy_json TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (scope_type, scope_id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_review_changes_repo_id ON review_changes(repository_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_review_patch_sets_change_id ON review_patch_sets(change_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_review_approvals_change_id ON review_approvals(change_id)",
        )
        .execute(&self.pool)
        .await?;

        // Normalized signal event log and weekly metric snapshots.
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS normalized_signals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repository_id INTEGER NOT NULL,
                signal_type TEXT NOT NULL,
                source_key TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                ingested_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (repository_id) REFERENCES repositories(id),
                UNIQUE(repository_id, signal_type, source_key)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS weekly_metric_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repository_id INTEGER NOT NULL,
                week_start_utc TEXT NOT NULL,
                metric_version TEXT NOT NULL,
                metrics_json TEXT NOT NULL,
                window_days INTEGER NOT NULL DEFAULT 7,
                computed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (repository_id) REFERENCES repositories(id),
                UNIQUE(repository_id, week_start_utc, metric_version)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_normalized_signals_repo_time ON normalized_signals(repository_id, occurred_at)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_weekly_snapshots_repo_week ON weekly_metric_snapshots(repository_id, week_start_utc)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // ========== User Operations ==========

    pub async fn create_user(&self, username: &str, email: Option<&str>) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO users (username, email)
            VALUES (?, ?)
            "#,
        )
        .bind(username)
        .bind(email)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn get_user_by_username(&self, username: &str) -> Result<UserRow> {
        sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, username, email, github_token, github_login, created_at
            FROM users
            WHERE username = ?
            "#,
        )
        .bind(username)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => {
                DatabaseError::NotFound(format!("User '{}' not found", username))
            }
            e => DatabaseError::Query(e),
        })
    }

    pub async fn get_user_by_id(&self, id: i64) -> Result<UserRow> {
        sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, username, email, github_token, github_login, created_at
            FROM users
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => {
                DatabaseError::NotFound(format!("User with id {} not found", id))
            }
            e => DatabaseError::Query(e),
        })
    }

    pub async fn list_users(&self) -> Result<Vec<UserRow>> {
        sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, username, email, github_token, github_login, created_at
            FROM users
            ORDER BY username
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(DatabaseError::Query)
    }

    /// Update the GitHub Personal Access Token for a user.
    /// Pass `None` to clear the token.
    pub async fn update_user_github_token(&self, user_id: i64, token: Option<&str>) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE users
            SET github_token = ?
            WHERE id = ?
            "#,
        )
        .bind(token)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Update the GitHub login (username on GitHub) for a user.
    /// Pass `None` to clear it.
    pub async fn update_user_github_login(
        &self,
        user_id: i64,
        github_login: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE users
            SET github_login = ?
            WHERE id = ?
            "#,
        )
        .bind(github_login)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Look up the GitHub token for the owner of the project that
    /// contains the given repository.
    ///
    /// Returns `None` when the repository or its owner has no token set.
    pub async fn get_github_token_for_repository(
        &self,
        repository_id: i64,
    ) -> Result<Option<String>> {
        #[derive(Debug, Clone, sqlx::FromRow)]
        struct TokenRow {
            github_token: Option<String>,
        }

        let result = sqlx::query_as::<_, TokenRow>(
            r#"
            SELECT u.github_token
            FROM repositories r
            JOIN projects p ON r.project_id = p.id
            JOIN users u ON p.owner_id = u.id
            WHERE r.id = ?
            "#,
        )
        .bind(repository_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DatabaseError::Query)?;

        Ok(result.and_then(|r| r.github_token))
    }

    // ========== Project Operations ==========

    pub async fn create_project(
        &self,
        name: &str,
        owner_id: i64,
        description: Option<&str>,
    ) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO projects (name, owner_id, description)
            VALUES (?, ?, ?)
            "#,
        )
        .bind(name)
        .bind(owner_id)
        .bind(description)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn get_project(&self, owner_id: i64, project_name: &str) -> Result<ProjectRow> {
        sqlx::query_as::<_, ProjectRow>(
            r#"
            SELECT id, name, owner_id, description, created_at
            FROM projects
            WHERE owner_id = ? AND name = ?
            "#,
        )
        .bind(owner_id)
        .bind(project_name)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => DatabaseError::NotFound(format!(
                "Project '{}' not found for owner_id {}",
                project_name, owner_id
            )),
            e => DatabaseError::Query(e),
        })
    }

    pub async fn list_projects_by_owner(&self, owner_id: i64) -> Result<Vec<ProjectRow>> {
        sqlx::query_as::<_, ProjectRow>(
            r#"
            SELECT id, name, owner_id, description, created_at
            FROM projects
            WHERE owner_id = ?
            ORDER BY name
            "#,
        )
        .bind(owner_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DatabaseError::Query)
    }

    // ========== Repository Operations ==========

    pub async fn create_repository(
        &self,
        project_id: i64,
        name: &str,
        git_url: &str,
    ) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO repositories (project_id, name, git_url)
            VALUES (?, ?, ?)
            "#,
        )
        .bind(project_id)
        .bind(name)
        .bind(git_url)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn get_repository(&self, project_id: i64, repo_name: &str) -> Result<RepositoryRow> {
        sqlx::query_as::<_, RepositoryRow>(
            r#"
            SELECT id, project_id, name, git_url, created_at
            FROM repositories
            WHERE project_id = ? AND name = ?
            "#,
        )
        .bind(project_id)
        .bind(repo_name)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => DatabaseError::NotFound(format!(
                "Repository '{}' not found in project_id {}",
                repo_name, project_id
            )),
            e => DatabaseError::Query(e),
        })
    }

    pub async fn get_repository_by_id(&self, repository_id: i64) -> Result<RepositoryRow> {
        sqlx::query_as::<_, RepositoryRow>(
            r#"
            SELECT id, project_id, name, git_url, created_at
            FROM repositories
            WHERE id = ?
            "#,
        )
        .bind(repository_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => {
                DatabaseError::NotFound(format!("Repository with id {} not found", repository_id))
            }
            e => DatabaseError::Query(e),
        })
    }

    pub async fn list_repositories_by_project(
        &self,
        project_id: i64,
    ) -> Result<Vec<RepositoryRow>> {
        sqlx::query_as::<_, RepositoryRow>(
            r#"
            SELECT id, project_id, name, git_url, created_at
            FROM repositories
            WHERE project_id = ?
            ORDER BY name
            "#,
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DatabaseError::Query)
    }

    pub async fn list_all_repositories(&self) -> Result<Vec<RepositoryRow>> {
        sqlx::query_as::<_, RepositoryRow>(
            r#"
            SELECT id, project_id, name, git_url, created_at
            FROM repositories
            ORDER BY project_id, name
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(DatabaseError::Query)
    }

    // ========== GitHub Operations ==========

    pub async fn create_github_pull_request(
        &self,
        github_id: i64,
        repository_id: i64,
        number: i32,
        title: &str,
        author_id: Option<i64>,
        state: &str,
        created_at: &str,
        updated_at: &str,
        closed_at: Option<&str>,
        merged_at: Option<&str>,
        additions: u32,
        deletions: u32,
        changed_files: u32,
    ) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO github_pull_requests (
                github_id, repository_id, number, title, author_id, state,
                created_at, updated_at, closed_at, merged_at, additions, deletions, changed_files
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(github_id)
        .bind(repository_id)
        .bind(number)
        .bind(title)
        .bind(author_id)
        .bind(state)
        .bind(created_at)
        .bind(updated_at)
        .bind(closed_at)
        .bind(merged_at)
        .bind(additions as i32)
        .bind(deletions as i32)
        .bind(changed_files as i32)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn create_github_review(
        &self,
        github_id: i64,
        pull_request_id: i64,
        user_id: Option<i64>,
        state: &str,
        submitted_at: &str,
    ) -> Result<i64> {
        let result = sqlx::query(
            r#"
             INSERT INTO github_reviews (
                 github_id, pull_request_id, user_id, state, submitted_at
             )
             VALUES (?, ?, ?, ?, ?)
             "#,
        )
        .bind(github_id)
        .bind(pull_request_id)
        .bind(user_id)
        .bind(state)
        .bind(submitted_at)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn create_github_commit(
        &self,
        github_sha: &str,
        repository_id: i64,
        author_name: Option<&str>,
        author_email: Option<&str>,
        committer_name: Option<&str>,
        committer_email: Option<&str>,
        message: &str,
        timestamp: &str,
    ) -> Result<i64> {
        let result = sqlx::query(
            r#"
             INSERT INTO github_commits (
                 github_sha, repository_id, author_name, author_email,
                 committer_name, committer_email, message, timestamp
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             "#,
        )
        .bind(github_sha)
        .bind(repository_id)
        .bind(author_name)
        .bind(author_email)
        .bind(committer_name)
        .bind(committer_email)
        .bind(message)
        .bind(timestamp)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn create_github_deployment(
        &self,
        github_id: i64,
        repository_id: i64,
        sha: &str,
        r#ref: &str,
        task: &str,
        payload: Option<&str>,
        environment: &str,
        state: &str,
        created_at: &str,
        updated_at: &str,
        creator_id: Option<i64>,
    ) -> Result<i64> {
        let result = sqlx::query(
            r#"
             INSERT INTO github_deployments (
                 github_id, repository_id, sha, ref, task, payload, environment,
                 state, created_at, updated_at, creator_id
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             "#,
        )
        .bind(github_id)
        .bind(repository_id)
        .bind(sha)
        .bind(r#ref)
        .bind(task)
        .bind(payload)
        .bind(environment)
        .bind(state)
        .bind(created_at)
        .bind(updated_at)
        .bind(creator_id)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn create_github_issue(
        &self,
        github_id: i64,
        repository_id: i64,
        number: i32,
        title: &str,
        author_id: Option<i64>,
        state: &str,
        created_at: &str,
        updated_at: &str,
        closed_at: Option<&str>,
    ) -> Result<i64> {
        let result = sqlx::query(
            r#"
             INSERT INTO github_issues (
                 github_id, repository_id, number, title, author_id, state,
                 created_at, updated_at, closed_at
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             "#,
        )
        .bind(github_id)
        .bind(repository_id)
        .bind(number)
        .bind(title)
        .bind(author_id)
        .bind(state)
        .bind(created_at)
        .bind(updated_at)
        .bind(closed_at)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn create_github_issue_label(
        &self,
        issue_id: i64,
        label_id: i64,
        label_name: &str,
        label_color: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
             INSERT INTO github_issue_labels (
                 issue_id, label_id, label_name, label_color
             )
             VALUES (?, ?, ?, ?)
             "#,
        )
        .bind(issue_id)
        .bind(label_id)
        .bind(label_name)
        .bind(label_color)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // ========== GitHub Query Operations ==========

    pub async fn list_github_pull_requests_by_repository(
        &self,
        repository_id: i64,
    ) -> Result<Vec<GithubPullRequestRow>> {
        sqlx::query_as::<_, GithubPullRequestRow>(
             r#"
             SELECT id, github_id, repository_id, number, title, author_id, state,
                    created_at, updated_at, closed_at, merged_at, additions, deletions, changed_files
             FROM github_pull_requests
             WHERE repository_id = ?
             ORDER BY number
             "#,
         )
         .bind(repository_id)
         .fetch_all(&self.pool)
         .await
         .map_err(DatabaseError::Query)
    }

    pub async fn upsert_normalized_pull_requests(
        &self,
        pull_requests: &[NormalizedPullRequest],
    ) -> Result<()> {
        for pull_request in pull_requests {
            self.upsert_normalized_signal(
                pull_request.repository_id,
                "pull_request",
                &format!("{}", pull_request.id),
                &pull_request.opened_at().to_rfc3339(),
                pull_request,
            )
            .await?;
        }

        Ok(())
    }

    pub async fn upsert_normalized_reviews(&self, reviews: &[NormalizedReview]) -> Result<()> {
        for review in reviews {
            let repository_id = self
                .resolve_repository_id_for_review(review.pull_request_id)
                .await?;

            self.upsert_normalized_signal(
                repository_id,
                "review",
                &format!("{}", review.id),
                &review.submitted_timestamp().to_rfc3339(),
                review,
            )
            .await?;
        }

        Ok(())
    }

    pub async fn upsert_normalized_commits(&self, commits: &[NormalizedCommit]) -> Result<()> {
        for commit in commits {
            self.upsert_normalized_signal(
                commit.repository_id,
                "commit",
                &commit.sha,
                &commit.committed_timestamp().to_rfc3339(),
                commit,
            )
            .await?;
        }

        Ok(())
    }

    pub async fn upsert_normalized_deployments(
        &self,
        deployments: &[NormalizedDeployment],
    ) -> Result<()> {
        for deployment in deployments {
            self.upsert_normalized_signal(
                deployment.repository_id,
                "deployment",
                &format!("{}", deployment.id),
                &deployment.deployed_at().to_rfc3339(),
                deployment,
            )
            .await?;
        }

        Ok(())
    }

    pub async fn upsert_normalized_issues(&self, issues: &[NormalizedIssue]) -> Result<()> {
        for issue in issues {
            self.upsert_normalized_signal(
                issue.repository_id,
                "issue",
                &format!("{}", issue.id),
                &issue.opened_at().to_rfc3339(),
                issue,
            )
            .await?;
        }

        Ok(())
    }

    pub async fn list_normalized_signals_by_repository(
        &self,
        repository_id: i64,
        limit: i64,
    ) -> Result<Vec<NormalizedSignalRow>> {
        sqlx::query_as::<_, NormalizedSignalRow>(
            r#"
            SELECT
                id,
                repository_id,
                signal_type,
                source_key,
                occurred_at,
                payload_json,
                ingested_at,
                updated_at
            FROM normalized_signals
            WHERE repository_id = ?
            ORDER BY occurred_at DESC
            LIMIT ?
            "#,
        )
        .bind(repository_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(DatabaseError::Query)
    }

    pub async fn upsert_weekly_metric_snapshot(
        &self,
        repository_id: i64,
        week_start_utc: &str,
        metric_version: &str,
        metrics_json: &str,
        computed_at: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO weekly_metric_snapshots (
                repository_id, week_start_utc, metric_version, metrics_json, window_days, computed_at
            )
            VALUES (?, ?, ?, ?, 7, ?)
            ON CONFLICT(repository_id, week_start_utc, metric_version)
            DO UPDATE SET
                metrics_json = excluded.metrics_json,
                window_days = excluded.window_days,
                computed_at = excluded.computed_at,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(repository_id)
        .bind(week_start_utc)
        .bind(metric_version)
        .bind(metrics_json)
        .bind(computed_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn list_weekly_metric_snapshots_by_repository(
        &self,
        repository_id: i64,
        limit: i64,
    ) -> Result<Vec<WeeklyMetricSnapshotRow>> {
        sqlx::query_as::<_, WeeklyMetricSnapshotRow>(
            r#"
            SELECT
                id,
                repository_id,
                week_start_utc,
                metric_version,
                metrics_json,
                window_days,
                computed_at,
                updated_at
            FROM weekly_metric_snapshots
            WHERE repository_id = ?
            ORDER BY week_start_utc DESC
            LIMIT ?
            "#,
        )
        .bind(repository_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(DatabaseError::Query)
    }

    pub async fn list_weekly_metric_snapshots_in_rolling_window(
        &self,
        repository_id: i64,
        window_start_utc: &str,
        window_end_utc: &str,
    ) -> Result<Vec<WeeklyMetricSnapshotRow>> {
        sqlx::query_as::<_, WeeklyMetricSnapshotRow>(
            r#"
            SELECT
                id,
                repository_id,
                week_start_utc,
                metric_version,
                metrics_json,
                window_days,
                computed_at,
                updated_at
            FROM weekly_metric_snapshots
            WHERE repository_id = ?
              AND week_start_utc >= ?
              AND week_start_utc <= ?
            ORDER BY week_start_utc DESC
            "#,
        )
        .bind(repository_id)
        .bind(window_start_utc)
        .bind(window_end_utc)
        .fetch_all(&self.pool)
        .await
        .map_err(DatabaseError::Query)
    }

    async fn upsert_normalized_signal<T: Serialize>(
        &self,
        repository_id: i64,
        signal_type: &str,
        source_key: &str,
        occurred_at: &str,
        payload: &T,
    ) -> Result<()> {
        let payload_json = serde_json::to_string(payload)
            .map_err(|error| DatabaseError::InvalidData(error.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO normalized_signals (
                repository_id, signal_type, source_key, occurred_at, payload_json
            )
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(repository_id, signal_type, source_key)
            DO UPDATE SET
                occurred_at = excluded.occurred_at,
                payload_json = excluded.payload_json,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(repository_id)
        .bind(signal_type)
        .bind(source_key)
        .bind(occurred_at)
        .bind(payload_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn resolve_repository_id_for_review(&self, pull_request_id: i64) -> Result<i64> {
        let row = sqlx::query_as::<_, (i64,)>(
            r#"
            SELECT repository_id
            FROM github_pull_requests
            WHERE github_id = ?
            LIMIT 1
            "#,
        )
        .bind(pull_request_id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|tuple| tuple.0).ok_or_else(|| {
            DatabaseError::NotFound(format!(
                "Repository not found for pull request github_id {}",
                pull_request_id
            ))
        })
    }

    // ========== Project Members Operations ==========

    pub async fn add_project_member(
        &self,
        project_id: i64,
        user_id: i64,
        role: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO project_members (project_id, user_id, role)
            VALUES (?, ?, ?)
            "#,
        )
        .bind(project_id)
        .bind(user_id)
        .bind(role)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn list_project_members(&self, project_id: i64) -> Result<Vec<ProjectMemberRow>> {
        sqlx::query_as::<_, ProjectMemberRow>(
            r#"
            SELECT project_id, user_id, role, created_at
            FROM project_members
            WHERE project_id = ?
            "#,
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DatabaseError::Query)
    }

    // ========== Review Operations ==========

    pub async fn create_review_change(
        &self,
        repository_id: i64,
        change_key: &str,
        target_branch: &str,
        subject: &str,
        owner_user_id: i64,
        status: &str,
        current_patch_set: i32,
    ) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO review_changes (
                repository_id, change_key, target_branch, subject,
                owner_user_id, status, current_patch_set
            )
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(repository_id)
        .bind(change_key)
        .bind(target_branch)
        .bind(subject)
        .bind(owner_user_id)
        .bind(status)
        .bind(current_patch_set)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn get_review_change(&self, change_id: i64) -> Result<ReviewChangeRow> {
        sqlx::query_as::<_, ReviewChangeRow>(
            r#"
            SELECT
                id, repository_id, change_key, target_branch, subject,
                owner_user_id, status, current_patch_set, created_at, updated_at
            FROM review_changes
            WHERE id = ?
            "#,
        )
        .bind(change_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => {
                DatabaseError::NotFound(format!("Review change with id {} not found", change_id))
            }
            e => DatabaseError::Query(e),
        })
    }

    pub async fn update_review_change_status(&self, change_id: i64, status: &str) -> Result<()> {
        let result = sqlx::query(
            r#"
            UPDATE review_changes
            SET status = ?, updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            "#,
        )
        .bind(status)
        .bind(change_id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DatabaseError::NotFound(format!(
                "Review change with id {} not found",
                change_id
            )));
        }

        Ok(())
    }

    pub async fn list_review_changes_by_repository(
        &self,
        repository_id: i64,
    ) -> Result<Vec<ReviewChangeRow>> {
        sqlx::query_as::<_, ReviewChangeRow>(
            r#"
            SELECT
                id, repository_id, change_key, target_branch, subject,
                owner_user_id, status, current_patch_set, created_at, updated_at
            FROM review_changes
            WHERE repository_id = ?
            ORDER BY id DESC
            "#,
        )
        .bind(repository_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DatabaseError::Query)
    }

    pub async fn append_review_patch_set(
        &self,
        change_id: i64,
        number: i32,
        revision: &str,
        kind: &str,
        uploader_user_id: i64,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO review_patch_sets (
                change_id, number, revision, kind, uploader_user_id
            )
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(change_id)
        .bind(number)
        .bind(revision)
        .bind(kind)
        .bind(uploader_user_id)
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            UPDATE review_changes
            SET current_patch_set = ?, updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            "#,
        )
        .bind(number)
        .bind(change_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn list_review_patch_sets(&self, change_id: i64) -> Result<Vec<ReviewPatchSetRow>> {
        sqlx::query_as::<_, ReviewPatchSetRow>(
            r#"
            SELECT change_id, number, revision, kind, uploader_user_id, created_at
            FROM review_patch_sets
            WHERE change_id = ?
            ORDER BY number
            "#,
        )
        .bind(change_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DatabaseError::Query)
    }

    pub async fn upsert_review_approval(
        &self,
        change_id: i64,
        user_id: i64,
        label: &str,
        value: i32,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO review_approvals (change_id, user_id, label, value)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(change_id, user_id, label)
            DO UPDATE SET
                value = excluded.value,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(change_id)
        .bind(user_id)
        .bind(label)
        .bind(value)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn list_review_approvals(&self, change_id: i64) -> Result<Vec<ReviewApprovalRow>> {
        sqlx::query_as::<_, ReviewApprovalRow>(
            r#"
            SELECT change_id, user_id, label, value, updated_at
            FROM review_approvals
            WHERE change_id = ?
            ORDER BY label, user_id
            "#,
        )
        .bind(change_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DatabaseError::Query)
    }

    pub async fn set_review_policy_override(
        &self,
        scope_type: &str,
        scope_id: i64,
        policy_json: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO review_policy_overrides (scope_type, scope_id, policy_json)
            VALUES (?, ?, ?)
            ON CONFLICT(scope_type, scope_id)
            DO UPDATE SET
                policy_json = excluded.policy_json,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(scope_type)
        .bind(scope_id)
        .bind(policy_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_review_policy_override(
        &self,
        scope_type: &str,
        scope_id: i64,
    ) -> Result<Option<ReviewPolicyOverrideRow>> {
        sqlx::query_as::<_, ReviewPolicyOverrideRow>(
            r#"
            SELECT scope_type, scope_id, policy_json, updated_at
            FROM review_policy_overrides
            WHERE scope_type = ? AND scope_id = ?
            "#,
        )
        .bind(scope_type)
        .bind(scope_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DatabaseError::Query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    fn parse_ts(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    #[tokio::test]
    async fn initializes_snapshot_table() {
        let db = Database::new("sqlite::memory:").await.expect("db");

        let snapshots: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='weekly_metric_snapshots'",
        )
        .fetch_one(&*db)
        .await
        .expect("weekly snapshots table exists");

        assert_eq!(snapshots, 1);
    }

    #[tokio::test]
    async fn upserts_normalized_signals_and_weekly_snapshots() {
        let db = Database::new("sqlite::memory:").await.expect("db");

        let user_id = db
            .create_user("octocat", Some("octo@example.com"))
            .await
            .expect("user");
        let project_id = db
            .create_project("dash", user_id, None)
            .await
            .expect("project");
        let repository_id = db
            .create_repository(project_id, "repohub", "https://example.invalid/repo.git")
            .await
            .expect("repo");

        let pull_request = NormalizedPullRequest {
            id: 101,
            repository_id,
            number: 42,
            title: "Improve snapshot query".to_string(),
            author_id: user_id,
            state: "open".to_string(),
            created_at: parse_ts("2026-05-10T00:00:00Z"),
            updated_at: parse_ts("2026-05-10T00:10:00Z"),
            closed_at: None,
            merged_at: None,
            additions: 10,
            deletions: 2,
            changed_files: 1,
            head_sha: "abc123".to_string(),
            head_ref: "feature/snapshots".to_string(),
            base_sha: "def456".to_string(),
            base_ref: "main".to_string(),
            merge_commit_sha: None,
            draft: false,
            author_association: Some("CONTRIBUTOR".to_string()),
        };

        db.create_github_pull_request(
            pull_request.id,
            repository_id,
            pull_request.number,
            &pull_request.title,
            Some(pull_request.author_id),
            &pull_request.state,
            &pull_request.created_at.to_rfc3339(),
            &pull_request.updated_at.to_rfc3339(),
            None,
            None,
            pull_request.additions,
            pull_request.deletions,
            pull_request.changed_files,
        )
        .await
        .expect("create github pull request");

        db.upsert_normalized_pull_requests(&[pull_request.clone()])
            .await
            .expect("upsert pr");

        let review = NormalizedReview {
            id: 900,
            pull_request_id: pull_request.id,
            user_id,
            state: "APPROVED".to_string(),
            submitted_at: parse_ts("2026-05-10T01:00:00Z"),
            body: Some("LGTM".to_string()),
            commit_id: Some("abc123".to_string()),
        };

        db.upsert_normalized_reviews(&[review])
            .await
            .expect("upsert review");

        let rows = db
            .list_normalized_signals_by_repository(repository_id, 20)
            .await
            .expect("list signals");
        assert_eq!(rows.len(), 2);

        let pull_request_updated = NormalizedPullRequest {
            title: "Improve snapshot query v2".to_string(),
            ..pull_request
        };

        db.upsert_normalized_pull_requests(&[pull_request_updated])
            .await
            .expect("upsert pr again");

        let rows = db
            .list_normalized_signals_by_repository(repository_id, 20)
            .await
            .expect("list signals after update");
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .any(|row| row.payload_json.contains("snapshot query v2"))
        );

        db.upsert_weekly_metric_snapshot(
            repository_id,
            "2026-05-04T00:00:00Z",
            "v1",
            r#"{"deployment_frequency":3}"#,
            "2026-05-11T08:00:00Z",
        )
        .await
        .expect("insert weekly snapshot");

        db.upsert_weekly_metric_snapshot(
            repository_id,
            "2026-05-04T00:00:00Z",
            "v1",
            r#"{"deployment_frequency":4}"#,
            "2026-05-11T09:00:00Z",
        )
        .await
        .expect("update weekly snapshot");

        let snapshots = db
            .list_weekly_metric_snapshots_by_repository(repository_id, 10)
            .await
            .expect("list snapshots");

        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].window_days, 7);
        assert!(snapshots[0].metrics_json.contains("4"));

        let rolling = db
            .list_weekly_metric_snapshots_in_rolling_window(
                repository_id,
                "2026-05-01T00:00:00Z",
                "2026-05-10T00:00:00Z",
            )
            .await
            .expect("rolling window query");
        assert_eq!(rolling.len(), 1);
    }
}
