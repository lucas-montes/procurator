use serde::{Deserialize, Serialize};

use crate::domain::ProjectConfiguration;

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub email: Option<String>,
    pub github_token: Option<String>,
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

#[derive(Debug, Deserialize)]
pub struct UpdateGithubTokenRequest {
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SaveConfigurationRequest {
    pub configuration: ProjectConfiguration,
}

// GitHub API response DTOs
#[derive(Debug, Deserialize)]
pub struct GithubPullRequest {
    pub id: i64,
    pub number: i32,
    pub title: String,
    pub user: GithubUser,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub merged_at: Option<String>,
    pub additions: u32,
    pub deletions: u32,
    pub changed_files: u32,
    pub head: Option<GithubPullRequestBranch>,
    pub base: Option<GithubPullRequestBranch>,
    pub draft: Option<bool>,
    pub author_association: Option<String>,
    pub merge_commit_sha: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GithubPullRequestBranch {
    pub sha: String,
    #[serde(rename = "ref")]
    pub ref_field: String,
}

#[derive(Debug, Deserialize)]
pub struct GithubUser {
    pub id: i64,
    pub login: String,
}

#[derive(Debug, Deserialize)]
pub struct GithubReview {
    pub id: i64,
    pub user: GithubUser,
    pub state: String,
    pub submitted_at: String,
    pub body: Option<String>,
    pub commit_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GithubCommit {
    pub sha: String,
    pub commit: GithubCommitInfo,
    pub author: Option<GithubUser>,
    pub committer: Option<GithubUser>,
    pub stats: Option<GithubCommitStats>,
}

#[derive(Debug, Deserialize)]
pub struct GithubCommitInfo {
    pub author: GithubCommitAuthor,
    pub committer: GithubCommitAuthor,
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Deserialize)]
pub struct GithubCommitAuthor {
    pub name: String,
    pub email: String,
    pub date: String,
}

#[derive(Debug, Deserialize)]
pub struct GithubCommitStats {
    pub additions: u32,
    pub deletions: u32,
    pub total: u32,
}

#[derive(Debug, Deserialize)]
pub struct GithubDeployment {
    pub id: i64,
    pub sha: String,
    #[serde(rename = "ref")]
    pub ref_field: String,
    pub task: String,
    pub payload: Option<serde_json::Value>,
    pub environment: String,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
    pub creator: Option<GithubUser>,
    pub description: Option<String>,
    pub production_environment: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct GithubIssue {
    pub id: i64,
    pub number: i32,
    pub title: String,
    pub user: GithubUser,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub labels: Vec<GithubLabel>,
    pub pull_request: Option<serde_json::Value>,
    pub assignee: Option<GithubUser>,
    pub milestone: Option<GithubMilestone>,
}

#[derive(Debug, Deserialize)]
pub struct GithubMilestone {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct GithubLabel {
    pub id: i64,
    pub name: String,
    pub color: String,
}

/// Response from POST https://github.com/login/oauth/access_token
#[derive(Debug, Deserialize)]
pub struct GithubAccessTokenResponse {
    pub access_token: Option<String>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// Response from GET https://api.github.com/user
#[derive(Debug, Deserialize)]
pub struct GithubUserResponse {
    pub login: String,
}

/// Repo item from GET https://api.github.com/user/repos
#[derive(Debug, Serialize, Deserialize)]
pub struct GithubRepoItem {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub html_url: String,
    pub clone_url: String,
    pub private: bool,
    pub description: Option<String>,
}
