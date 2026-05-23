use std::{future::Future, pin::Pin};

use crate::domain::signals::{
    NormalizedCommit, NormalizedDeployment, NormalizedIssue, NormalizedPullRequest,
    NormalizedReview,
};

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeRepositoryTarget {
    pub repository_id: i64,
    pub owner: String,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub struct NormalizedSignalBatch {
    pub pull_requests: Vec<NormalizedPullRequest>,
    pub reviews: Vec<NormalizedReview>,
    pub commits: Vec<NormalizedCommit>,
    pub deployments: Vec<NormalizedDeployment>,
    pub issues: Vec<NormalizedIssue>,
}

#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("authentication failed: {0}")]
    Authentication(String),
    #[error("upstream forge error: {0}")]
    Upstream(String),
    #[error("signal normalization error: {0}")]
    Transform(String),
}

pub trait ForgeSignalPort: Send + Sync {
    fn fetch_pull_requests<'a>(
        &'a self,
        target: &'a ForgeRepositoryTarget,
    ) -> PortFuture<'a, Result<Vec<NormalizedPullRequest>, ForgeError>>;

    fn fetch_reviews<'a>(
        &'a self,
        target: &'a ForgeRepositoryTarget,
    ) -> PortFuture<'a, Result<Vec<NormalizedReview>, ForgeError>>;

    fn fetch_commits<'a>(
        &'a self,
        target: &'a ForgeRepositoryTarget,
    ) -> PortFuture<'a, Result<Vec<NormalizedCommit>, ForgeError>>;

    fn fetch_deployments<'a>(
        &'a self,
        target: &'a ForgeRepositoryTarget,
    ) -> PortFuture<'a, Result<Vec<NormalizedDeployment>, ForgeError>>;

    fn fetch_issues<'a>(
        &'a self,
        target: &'a ForgeRepositoryTarget,
    ) -> PortFuture<'a, Result<Vec<NormalizedIssue>, ForgeError>>;

    fn fetch_all<'a>(
        &'a self,
        target: &'a ForgeRepositoryTarget,
    ) -> PortFuture<'a, Result<NormalizedSignalBatch, ForgeError>>;
}
