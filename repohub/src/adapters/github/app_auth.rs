use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use octocrab::Octocrab;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GithubAppAuthError {
    #[error("jwt error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("parse error: {0}")]
    Parse(#[from] chrono::ParseError),
    #[error("octocrab error: {0}")]
    Octo(#[from] octocrab::Error),
}

#[derive(Serialize)]
struct Claims {
    iat: usize,
    exp: usize,
    iss: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    token: String,
    expires_at: String,
}

/// Thin authenticator for GitHub Apps. Responsible for creating the
/// short-lived app JWT and exchanging it for an installation access token.
#[derive(Clone)]
pub struct GithubAppAuthenticator {
    app_id: u64,
    private_key_pem: Vec<u8>,
    http: Client,
}

impl GithubAppAuthenticator {
    /// Create a new authenticator. `private_key_pem` should contain the PEM-encoded RSA private key.
    pub fn new(app_id: u64, private_key_pem: Vec<u8>) -> Self {
        Self {
            app_id,
            private_key_pem,
            http: Client::new(),
        }
    }

    /// Create a short-lived JWT for the GitHub App (RS256).
    pub fn create_jwt(&self) -> Result<String, GithubAppAuthError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs() as usize;
        let exp = now + 600; // 10 minutes

        let claims = Claims {
            iat: now,
            exp,
            iss: self.app_id.to_string(),
        };

        let encoding_key = EncodingKey::from_rsa_pem(&self.private_key_pem)?;
        let header = Header::new(Algorithm::RS256);
        let token = encode(&header, &claims, &encoding_key)?;
        Ok(token)
    }

    /// Exchange a JWT for an installation access token.
    pub async fn installation_token(
        &self,
        installation_id: u64,
    ) -> Result<(String, DateTime<Utc>), GithubAppAuthError> {
        let jwt = self.create_jwt()?;

        let url = format!(
            "https://api.github.com/app/installations/{}/access_tokens",
            installation_id
        );

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", jwt))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "repohub")
            .send()
            .await?
            .error_for_status()?;

        let body: TokenResponse = resp.json().await?;
        let expires_at = DateTime::parse_from_rfc3339(&body.expires_at)?.with_timezone(&Utc);

        Ok((body.token, expires_at))
    }

    /// Return an `octocrab::Octocrab` client authenticated as the installation.
    /// The returned client uses the installation access token; callers should recreate
    /// the client when the token expires.
    pub async fn installation_client(
        &self,
        installation_id: u64,
    ) -> Result<Octocrab, GithubAppAuthError> {
        let (token, _expires_at) = self.installation_token(installation_id).await?;
        let oc = Octocrab::builder().personal_token(token).build()?;
        Ok(oc)
    }
}
