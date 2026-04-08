use std::{future::Future, pin::Pin};

use crate::adapters::shared::database::{ProjectRow, RepositoryRow, UserRow};

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug)]
pub enum GithubError {
    NotFound(String),
    InvalidInput(String),
    Storage(String),
}

impl std::fmt::Display for GithubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(message) => write!(f, "Not found: {message}"),
            Self::InvalidInput(message) => write!(f, "Invalid input: {message}"),
            Self::Storage(message) => write!(f, "Storage error: {message}"),
        }
    }
}

impl std::error::Error for GithubError {}

pub trait GithubPort {
    fn list_users<'a>(&'a self) -> PortFuture<'a, Result<Vec<UserRow>, GithubError>>;
    fn create_user<'a>(
        &'a self,
        username: &'a str,
        email: Option<&'a str>,
    ) -> PortFuture<'a, Result<i64, GithubError>>;
    fn get_user_by_username<'a>(
        &'a self,
        username: &'a str,
    ) -> PortFuture<'a, Result<UserRow, GithubError>>;

    fn list_projects_by_owner<'a>(
        &'a self,
        owner_id: i64,
    ) -> PortFuture<'a, Result<Vec<ProjectRow>, GithubError>>;
    fn create_project<'a>(
        &'a self,
        name: &'a str,
        owner_id: i64,
        description: Option<&'a str>,
    ) -> PortFuture<'a, Result<i64, GithubError>>;
    fn get_project<'a>(
        &'a self,
        owner_id: i64,
        project_name: &'a str,
    ) -> PortFuture<'a, Result<ProjectRow, GithubError>>;

    fn list_repositories_by_project<'a>(
        &'a self,
        project_id: i64,
    ) -> PortFuture<'a, Result<Vec<RepositoryRow>, GithubError>>;
    fn create_repository<'a>(
        &'a self,
        project_id: i64,
        name: &'a str,
        git_url: &'a str,
    ) -> PortFuture<'a, Result<i64, GithubError>>;
    fn get_repository<'a>(
        &'a self,
        project_id: i64,
        repo_name: &'a str,
    ) -> PortFuture<'a, Result<RepositoryRow, GithubError>>;
}
