//! Domain Models
//!
//! Business entities that represent the core domain.
//! These are independent of the database layer.

pub mod configuration;

use serde::{Deserialize, Serialize};

use crate::database::{ProjectRow, RepositoryRow, UserRow};

pub use configuration::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub email: Option<String>,
    pub created_at: String,
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        Self {
            id: row.id,
            username: row.username,
            email: row.email,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub owner_id: i64,
    pub description: Option<String>,
    pub created_at: String,
}

impl From<ProjectRow> for Project {
    fn from(row: ProjectRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            owner_id: row.owner_id,
            description: row.description,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub git_url: String,
    pub created_at: String,
}

impl From<RepositoryRow> for Repository {
    fn from(row: RepositoryRow) -> Self {
        Self {
            id: row.id,
            project_id: row.project_id,
            name: row.name,
            git_url: row.git_url,
            created_at: row.created_at,
        }
    }
}

// DTOs for creating entities
#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRepositoryRequest {
    pub name: String,
    pub git_url: Option<String>,
}
