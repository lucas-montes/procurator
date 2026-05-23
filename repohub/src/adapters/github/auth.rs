/// GitHub authentication via Personal Access Token (PAT).
///
/// Users provide their own token (with `repo` scope) via the
/// `/{username}/github-token` endpoint. This token is stored in the
/// database and used for GitHub API calls when fetching DORA metrics.
#[derive(Debug, Clone)]
pub struct GithubAuth {
    token: String,
}

impl GithubAuth {
    /// Create a PAT-based authenticator.
    pub fn new(token: String) -> Self {
        Self { token }
    }

    /// Return the GitHub API token for use in `Authorization` headers.
    pub fn get_token(&self) -> String {
        self.token.clone()
    }
}
