use crate::adapters::github::auth::GithubAuth;
use crate::adapters::github::dto::{
    GithubCommit, GithubDeployment, GithubIssue, GithubPullRequest, GithubReview,
};
use crate::adapters::shared::database::Database;
use crate::application::ports::{
    ForgeError, ForgeRepositoryTarget, ForgeSignalPort, NormalizedSignalBatch, PortFuture,
};
use crate::domain::signals::{
    normalize_commit, normalize_deployment, normalize_issue, normalize_pull_request,
    normalize_review,
};
use thiserror::Error;
use tracing::{info, warn};

#[derive(Debug, Error)]
pub enum GithubClientError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),
    #[error("Failed to parse JSON response: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("GitHub API error: {0}")]
    ApiError(String),
    #[error("Authentication error: {0}")]
    AuthError(String),
}

impl From<GithubClientError> for ForgeError {
    fn from(value: GithubClientError) -> Self {
        match value {
            GithubClientError::ApiError(error) => ForgeError::Upstream(error),
            GithubClientError::HttpError(error) => ForgeError::Upstream(error.to_string()),
            GithubClientError::JsonError(error) => ForgeError::Upstream(error.to_string()),
        }
    }
}

/// GitHub API client for fetching repository data
pub struct GithubClient {
    auth: GithubAuth,
    db: Database,
    owner: String,
    repo: String,
}

impl GithubClient {
    pub fn new(auth: GithubAuth, db: Database, owner: String, repo: String) -> Self {
        Self {
            auth,
            db,
            owner,
            repo,
        }
    }

    /// Base URL for GitHub API requests
    fn api_url(&self, path: &str) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/{}",
            self.owner, self.repo, path
        )
    }

    fn validate_target(&self, target: &ForgeRepositoryTarget) -> Result<(), ForgeError> {
        if target.owner != self.owner || target.name != self.repo {
            return Err(ForgeError::InvalidInput(format!(
                "client configured for {}/{} but received target {}/{}",
                self.owner, self.repo, target.owner, target.name
            )));
        }

        Ok(())
    }

    /// Fetch all pull requests for the repository (with pagination)
    pub async fn fetch_pull_requests(&self) -> Result<Vec<GithubPullRequest>, GithubClientError> {
        let token = self
            .auth
            .get_token()
            .await
            .map_err(|e| GithubClientError::AuthError(e.to_string()))?;
        let client = reqwest::Client::new();

        let mut all_prs = Vec::new();
        let mut page = 1;
        let per_page = 100;

        loop {
            let url = format!(
                "{}?state=all&per_page={}&page={}",
                self.api_url("pulls"),
                per_page,
                page
            );

            let response = client
                .get(&url)
                .header("Authorization", format!("token {}", token))
                .header("Accept", "application/vnd.github+json")
                .send()
                .await?;

            if !response.status().is_success() {
                return Err(GithubClientError::ApiError(format!(
                    "Failed to fetch PRs: {}",
                    response.status()
                )));
            }

            let prs: Vec<GithubPullRequest> = response.json().await?;
            if prs.is_empty() {
                break;
            }

            all_prs.extend(prs);
            page += 1;
        }

        info!(
            "Fetched {} pull requests for {}/{}",
            all_prs.len(),
            self.owner,
            self.repo
        );
        Ok(all_prs)
    }

    async fn fetch_reviews_for_pull_request(
        &self,
        token: &str,
        pull_request_number: i32,
    ) -> Result<Vec<GithubReview>, GithubClientError> {
        let client = reqwest::Client::new();
        let mut all_reviews = Vec::new();
        let mut page = 1;
        let per_page = 100;

        loop {
            let url = format!(
                "{}pulls/{}/reviews?per_page={}&page={}",
                self.api_url(""),
                pull_request_number,
                per_page,
                page
            );

            let response = client
                .get(&url)
                .header("Authorization", format!("token {}", token))
                .header("Accept", "application/vnd.github+json")
                .send()
                .await?;

            if !response.status().is_success() {
                warn!(
                    "Failed to fetch reviews for PR {}: {}",
                    pull_request_number,
                    response.status()
                );
                break;
            }

            let reviews: Vec<GithubReview> = response.json().await?;
            if reviews.is_empty() {
                break;
            }

            all_reviews.extend(reviews);
            page += 1;
        }

        Ok(all_reviews)
    }

    /// Fetch all reviews for pull requests in the repository
    pub async fn fetch_pull_request_reviews(&self) -> Result<Vec<GithubReview>, GithubClientError> {
        let token = self
            .auth
            .get_token()
            .await
            .map_err(|e| GithubClientError::AuthError(e.to_string()))?;

        // First get all PR numbers to fetch reviews for each
        let prs = self.fetch_pull_requests().await?;
        let mut all_reviews = Vec::new();

        for pr in prs {
            let reviews = self
                .fetch_reviews_for_pull_request(&token, pr.number)
                .await?;
            all_reviews.extend(reviews);
        }

        info!(
            "Fetched {} reviews for {}/{}",
            all_reviews.len(),
            self.owner,
            self.repo
        );
        Ok(all_reviews)
    }

    /// Fetch all commits for the repository's default branch
    pub async fn fetch_commits(&self) -> Result<Vec<GithubCommit>, GithubClientError> {
        let token = self
            .auth
            .get_token()
            .await
            .map_err(|e| GithubClientError::AuthError(e.to_string()))?;
        let client = reqwest::Client::new();

        let mut all_commits = Vec::new();
        let mut page = 1;
        let per_page = 100;

        loop {
            let url = format!(
                "{}?per_page={}&page={}",
                self.api_url("commits"),
                per_page,
                page
            );

            let response = client
                .get(&url)
                .header("Authorization", format!("token {}", token))
                .header("Accept", "application/vnd.github+json")
                .send()
                .await?;

            if !response.status().is_success() {
                return Err(GithubClientError::ApiError(format!(
                    "Failed to fetch commits: {}",
                    response.status()
                )));
            }

            let commits: Vec<GithubCommit> = response.json().await?;
            if commits.is_empty() {
                break;
            }

            all_commits.extend(commits);
            page += 1;
        }

        info!(
            "Fetched {} commits for {}/{}",
            all_commits.len(),
            self.owner,
            self.repo
        );
        Ok(all_commits)
    }

    /// Fetch all deployments for the repository
    pub async fn fetch_deployments(&self) -> Result<Vec<GithubDeployment>, GithubClientError> {
        let token = self
            .auth
            .get_token()
            .await
            .map_err(|e| GithubClientError::AuthError(e.to_string()))?;
        let client = reqwest::Client::new();

        let mut all_deployments = Vec::new();
        let mut page = 1;
        let per_page = 100;

        loop {
            let url = format!(
                "{}?per_page={}&page={}",
                self.api_url("deployments"),
                per_page,
                page
            );

            let response = client
                .get(&url)
                .header("Authorization", format!("token {}", token))
                .header("Accept", "application/vnd.github+json")
                .send()
                .await?;

            if !response.status().is_success() {
                return Err(GithubClientError::ApiError(format!(
                    "Failed to fetch deployments: {}",
                    response.status()
                )));
            }

            let deployments: Vec<GithubDeployment> = response.json().await?;
            if deployments.is_empty() {
                break;
            }

            all_deployments.extend(deployments);
            page += 1;
        }

        info!(
            "Fetched {} deployments for {}/{}",
            all_deployments.len(),
            self.owner,
            self.repo
        );
        Ok(all_deployments)
    }

    /// Fetch all issues (including incidents) for the repository
    pub async fn fetch_issues(&self) -> Result<Vec<GithubIssue>, GithubClientError> {
        let token = self
            .auth
            .get_token()
            .await
            .map_err(|e| GithubClientError::AuthError(e.to_string()))?;
        let client = reqwest::Client::new();

        let mut all_issues = Vec::new();
        let mut page = 1;
        let per_page = 100;

        loop {
            let url = format!(
                "{}?state=all&per_page={}&page={}",
                self.api_url("issues"),
                per_page,
                page
            );

            let response = client
                .get(&url)
                .header("Authorization", format!("token {}", token))
                .header("Accept", "application/vnd.github+json")
                .send()
                .await?;

            if !response.status().is_success() {
                return Err(GithubClientError::ApiError(format!(
                    "Failed to fetch issues: {}",
                    response.status()
                )));
            }

            let issues: Vec<GithubIssue> = response.json().await?;
            if issues.is_empty() {
                break;
            }

            all_issues.extend(issues);
            page += 1;
        }

        info!(
            "Fetched {} issues for {}/{}",
            all_issues.len(),
            self.owner,
            self.repo
        );
        Ok(all_issues)
    }

    /// Fetch all data types and persist them to database
    pub async fn fetch_and_persist_all(&self) -> Result<(), GithubClientError> {
        let _ = &self.db;

        info!(
            "Starting GitHub data fetch for {}/{}",
            self.owner, self.repo
        );

        // Fetch all data types
        let prs = self.fetch_pull_requests().await?;
        let reviews = self.fetch_pull_request_reviews().await?;
        let commits = self.fetch_commits().await?;
        let deployments = self.fetch_deployments().await?;
        let issues = self.fetch_issues().await?;

        // TODO: Persist to database - this will be implemented in persistence module
        // For now, just log the counts
        info!(
            "Fetched data for {}/{} - PRs: {}, Reviews: {}, Commits: {}, Deployments: {}, Issues: {}",
            self.owner,
            self.repo,
            prs.len(),
            reviews.len(),
            commits.len(),
            deployments.len(),
            issues.len()
        );

        Ok(())
    }
}

impl ForgeSignalPort for GithubClient {
    fn fetch_pull_requests<'a>(
        &'a self,
        target: &'a ForgeRepositoryTarget,
    ) -> PortFuture<'a, Result<Vec<crate::domain::signals::NormalizedPullRequest>, ForgeError>>
    {
        Box::pin(async move {
            self.validate_target(target)?;
            let pull_requests = GithubClient::fetch_pull_requests(self)
                .await
                .map_err(ForgeError::from)?;
            pull_requests
                .iter()
                .map(|pr| {
                    normalize_pull_request(target.repository_id, pr)
                        .map_err(|error| ForgeError::Transform(error.to_string()))
                })
                .collect()
        })
    }

    fn fetch_reviews<'a>(
        &'a self,
        target: &'a ForgeRepositoryTarget,
    ) -> PortFuture<'a, Result<Vec<crate::domain::signals::NormalizedReview>, ForgeError>> {
        Box::pin(async move {
            self.validate_target(target)?;

            let token = self.auth.get_token();

            let pull_requests = GithubClient::fetch_pull_requests(self)
                .await
                .map_err(ForgeError::from)?;
            let mut normalized = Vec::new();

            for pull_request in pull_requests {
                let reviews = self
                    .fetch_reviews_for_pull_request(&token, pull_request.number)
                    .await
                    .map_err(ForgeError::from)?;

                for review in reviews {
                    let review = normalize_review(pull_request.id, &review)
                        .map_err(|error| ForgeError::Transform(error.to_string()))?;
                    normalized.push(review);
                }
            }

            Ok(normalized)
        })
    }

    fn fetch_commits<'a>(
        &'a self,
        target: &'a ForgeRepositoryTarget,
    ) -> PortFuture<'a, Result<Vec<crate::domain::signals::NormalizedCommit>, ForgeError>> {
        Box::pin(async move {
            self.validate_target(target)?;
            let commits = GithubClient::fetch_commits(self)
                .await
                .map_err(ForgeError::from)?;
            commits
                .iter()
                .map(|commit| {
                    normalize_commit(target.repository_id, commit)
                        .map_err(|error| ForgeError::Transform(error.to_string()))
                })
                .collect()
        })
    }

    fn fetch_deployments<'a>(
        &'a self,
        target: &'a ForgeRepositoryTarget,
    ) -> PortFuture<'a, Result<Vec<crate::domain::signals::NormalizedDeployment>, ForgeError>> {
        Box::pin(async move {
            self.validate_target(target)?;
            let deployments = GithubClient::fetch_deployments(self)
                .await
                .map_err(ForgeError::from)?;
            deployments
                .iter()
                .map(|deployment| {
                    normalize_deployment(target.repository_id, deployment)
                        .map_err(|error| ForgeError::Transform(error.to_string()))
                })
                .collect()
        })
    }

    fn fetch_issues<'a>(
        &'a self,
        target: &'a ForgeRepositoryTarget,
    ) -> PortFuture<'a, Result<Vec<crate::domain::signals::NormalizedIssue>, ForgeError>> {
        Box::pin(async move {
            self.validate_target(target)?;
            let issues = GithubClient::fetch_issues(self)
                .await
                .map_err(ForgeError::from)?;
            issues
                .iter()
                .map(|issue| {
                    normalize_issue(target.repository_id, issue)
                        .map_err(|error| ForgeError::Transform(error.to_string()))
                })
                .collect()
        })
    }

    fn fetch_all<'a>(
        &'a self,
        target: &'a ForgeRepositoryTarget,
    ) -> PortFuture<'a, Result<NormalizedSignalBatch, ForgeError>> {
        Box::pin(async move {
            self.validate_target(target)?;

            let pull_requests = ForgeSignalPort::fetch_pull_requests(self, target).await?;
            let reviews = ForgeSignalPort::fetch_reviews(self, target).await?;
            let commits = ForgeSignalPort::fetch_commits(self, target).await?;
            let deployments = ForgeSignalPort::fetch_deployments(self, target).await?;
            let issues = ForgeSignalPort::fetch_issues(self, target).await?;

            Ok(NormalizedSignalBatch {
                pull_requests,
                reviews,
                commits,
                deployments,
                issues,
            })
        })
    }
}
