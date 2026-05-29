use std::sync::Arc;

use chrono::{DateTime, Utc};
use octocrab::Octocrab;
use tokio::sync::Mutex;

use crate::adapters::github::app_auth::{GithubAppAuthError, GithubAppAuthenticator};
use crate::adapters::github::dto::GithubRepoItem;
use thiserror::Error;

/// Thin abstraction over different GitHub authentication mechanisms.
/// Supports Personal Access Token (PAT) and GitHub App installation tokens.
#[derive(Debug)]
pub struct GithubAuth {
    inner: GithubAuthInner,
}

#[derive(Debug)]
enum GithubAuthInner {
    Pat(String),
    App {
        authenticator: Arc<GithubAppAuthenticator>,
        installation_id: u64,
        // cache token and expiry
        cache: Mutex<Option<(String, DateTime<Utc>)>>,
    },
}

#[derive(Debug, Error)]
pub enum GithubAuthError {
    #[error(transparent)]
    AppAuth(#[from] GithubAppAuthError),
    #[error(transparent)]
    Octocrab(#[from] octocrab::Error),
    #[error("no token available")]
    NoToken,
}

impl GithubAuth {
    /// Create a PAT-based authenticator.
    pub fn from_pat(token: String) -> Self {
        Self {
            inner: GithubAuthInner::Pat(token),
        }
    }

    /// Create a GitHub App-based authenticator using the provided `GithubAppAuthenticator`
    /// and an `installation_id` to exchange for installation tokens.
    pub fn from_app(authenticator: Arc<GithubAppAuthenticator>, installation_id: u64) -> Self {
        Self {
            inner: GithubAuthInner::App {
                authenticator,
                installation_id,
                cache: Mutex::new(None),
            },
        }
    }

    /// Asynchronously obtain a valid API token. For PAT this returns immediately.
    /// For App-based auth this will perform an exchange if the cached token is missing or expired.
    pub async fn get_token(&self) -> Result<String, GithubAuthError> {
        match &self.inner {
            GithubAuthInner::Pat(token) => Ok(token.clone()),
            GithubAuthInner::App {
                authenticator,
                installation_id,
                cache,
            } => {
                // Check cache
                {
                    let guard = cache.lock().await;
                    if let Some((tok, expires)) = &*guard {
                        if *expires > Utc::now() {
                            return Ok(tok.clone());
                        }
                    }
                }

                // Cache miss or expired — request a new token
                let (token, expires_at) =
                    authenticator.installation_token(*installation_id).await?;

                let mut guard = cache.lock().await;
                *guard = Some((token.clone(), expires_at));
                Ok(token)
            }
        }
    }

    /// Build an authenticated `octocrab::Octocrab` client for the current auth context.
    ///
    /// This keeps callers on a single abstraction: they can obtain a token when needed,
    /// or ask for a ready-to-use GitHub API client without handling JWT or installation
    /// token exchange details.
    pub async fn octocrab_client(&self) -> Result<Octocrab, GithubAuthError> {
        let token = self.get_token().await?;
        Ok(Octocrab::builder().personal_token(token).build()?)
    }

    /// List the authenticated user's GitHub repositories through the current auth context.
    ///
    /// This is the repo-import path used by the UI; it stays behind the auth abstraction so
    /// the caller does not need to care whether the backing credential is a PAT or an
    /// installation token.
    pub async fn list_authenticated_user_repositories(
        &self,
    ) -> Result<Vec<GithubRepoItem>, GithubAuthError> {
        let octocrab = self.octocrab_client().await?;

        // `octocrab` handles the `Authorization` header via the authenticated client.
        // The route matches the authenticated-user repo list currently used by the UI.
        let repos: Vec<GithubRepoItem> = octocrab
            .get(
                "/user/repos?per_page=100&sort=updated&type=all",
                None::<&()>,
            )
            .await?;

        Ok(repos)
    }
}

impl Clone for GithubAuth {
    fn clone(&self) -> Self {
        match &self.inner {
            GithubAuthInner::Pat(t) => GithubAuth::from_pat(t.clone()),
            GithubAuthInner::App {
                authenticator,
                installation_id,
                cache,
            } => {
                // recreate with same authenticator and installation id; cache starts empty in clone
                GithubAuth::from_app(authenticator.clone(), *installation_id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GithubAuth;

    #[test]
    fn pat_auth_returns_token() {
        let auth = GithubAuth::from_pat("ghp_test_token".to_string());

        let token = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async { auth.get_token().await.expect("token") });

        assert_eq!(token, "ghp_test_token");
    }

    #[test]
    fn pat_auth_builds_octocrab_client() {
        let auth = GithubAuth::from_pat("ghp_test_token".to_string());

        let _client = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async { auth.octocrab_client().await.expect("client") });
    }
}
