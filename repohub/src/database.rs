//! Database Infrastructure Layer
//!
//! Handles database connection, schema initialization, and provides
//! data access methods for users, projects, and repositories.

use std::{ops::Deref, str::FromStr};

use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
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
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

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
            SELECT id, username, email, created_at
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
            SELECT id, username, email, created_at
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
            SELECT id, username, email, created_at
            FROM users
            ORDER BY username
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(DatabaseError::Query)
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

    pub async fn get_repository(
        &self,
        project_id: i64,
        repo_name: &str,
    ) -> Result<RepositoryRow> {
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
}
