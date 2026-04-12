use serde::{Deserialize, Serialize};

use crate::domain::ProjectConfiguration;

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

#[derive(Debug, Serialize, Deserialize)]
pub struct SaveConfigurationRequest {
    pub configuration: ProjectConfiguration,
}
