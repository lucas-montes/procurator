use askama::Template;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose};
use rand::Rng;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{
    adapters::shared::database::Database,
    config::Config,
    domain::{Project, Repository, User},
    services::RepositoryService,
};
use repo_outils::nix::FlakeMetadata;

use super::dto::{
    CreateProjectRequest, CreateRepositoryRequest, CreateUserRequest, GithubAccessTokenResponse,
    GithubUserResponse, SaveConfigurationRequest, UpdateGithubTokenRequest,
};

#[derive(Clone)]
pub struct GithubAppState {
    pub db: Database,
    pub repo_service: RepositoryService,
    pub config: Config,
    pub oauth_nonces: Arc<Mutex<HashMap<String, (String, Instant)>>>,
    pub app_auth: Option<std::sync::Arc<crate::adapters::github::app_auth::GithubAppAuthenticator>>,
}

impl GithubAppState {
    pub fn new(db: Database, config: &Config) -> Self {
        // Optionally construct a GitHub App authenticator if configured
        let app_auth = match (
            config.github_app_id,
            config.github_app_private_key_pem.as_deref(),
        ) {
            (Some(app_id), Some(pem)) if !pem.is_empty() => Some(std::sync::Arc::new(
                crate::adapters::github::app_auth::GithubAppAuthenticator::new(
                    app_id,
                    pem.as_bytes().to_vec(),
                ),
            )),
            _ => None,
        };

        Self {
            db,
            repo_service: RepositoryService::new(config),
            config: config.clone(),
            oauth_nonces: Arc::new(Mutex::new(HashMap::new())),
            app_auth,
        }
    }
}

struct HtmlTemplate<T>(T);

impl<T: Template> IntoResponse for HtmlTemplate<T> {
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => {
                tracing::error!("Template error: {}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Template error: {}", err),
                )
                    .into_response()
            }
        }
    }
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    users: Vec<User>,
}

#[derive(Template)]
#[template(path = "user.html")]
struct UserTemplate {
    user: User,
    projects: Vec<Project>,
}

#[derive(Template)]
#[template(path = "project.html")]
struct ProjectTemplate {
    username: String,
    project: Project,
    repositories: Vec<Repository>,
}

#[derive(Template)]
#[template(path = "repository.html")]
struct RepositoryTemplate {
    username: String,
    project_name: String,
    repo: Repository,
}

#[derive(Template)]
#[template(path = "not_implemented.html")]
struct NotImplementedTemplate {
    feature: String,
    description: String,
    back_url: String,
}

#[derive(Template)]
#[template(path = "flake.html")]
struct FlakeTemplate {
    username: String,
    project_name: String,
    repo_name: String,
    flake_metadata: Option<FlakeMetadata>,
}

#[derive(Template)]
#[template(path = "configuration_v2.html")]
struct ConfigurationTemplate {
    username: String,
    project_name: String,
    repositories: Vec<Repository>,
    repositories_json: String,
}

#[derive(Template)]
#[template(path = "agents.html")]
struct AgentsTemplate {
    username: String,
    project_name: String,
    agents: Vec<AgentView>,
    runs: Vec<RunView>,
}

#[derive(Template)]
#[template(path = "documentation.html")]
struct DocumentationTemplate {
    username: String,
    project_name: String,
    sources: Vec<DocSourceView>,
    documents: Vec<DocumentView>,
}

#[derive(Template)]
#[template(path = "stats.html")]
struct StatsTemplate {
    username: String,
    project_name: String,
    kpis: Vec<KpiView>,
    build_series: Vec<ChartPoint>,
    failure_series: Vec<ChartPoint>,
    recent_runs: Vec<RunView>,
    monitors: Vec<MonitorView>,
    monitor_options: Vec<MonitorOptionView>,
}

#[derive(Template)]
#[template(path = "milestones.html")]
struct MilestonesTemplate {
    username: String,
    project_name: String,
    milestones: Vec<MilestoneView>,
}

#[derive(Clone)]
struct AgentView {
    name: String,
    status: String,
    description: String,
    last_run: String,
    next_run: String,
}

#[derive(Clone)]
struct RunView {
    name: String,
    status: String,
    duration: String,
    started_at: String,
}

#[derive(Clone)]
struct DocSourceView {
    name: String,
    location: String,
    updated_at: String,
    status: String,
}

#[derive(Clone)]
struct DocumentView {
    title: String,
    repo: String,
    updated_at: String,
    status: String,
}

#[derive(Clone)]
struct KpiView {
    label: String,
    value: String,
}

#[derive(Clone)]
struct ChartPoint {
    label: String,
    value: u32,
    width_pct: u8,
}

#[derive(Clone)]
struct MonitorView {
    name: String,
    target: String,
    status: String,
    latency_ms: u32,
    uptime: String,
    last_check: String,
    ssl_expires: String,
    regions: String,
    alerting: String,
}

#[derive(Clone)]
struct MonitorOptionView {
    name: String,
    description: String,
}

#[derive(Clone)]
struct MilestoneView {
    title: String,
    due_date: String,
    status: String,
    progress: u8,
    description: String,
}

async fn index(State(state): State<GithubAppState>) -> impl IntoResponse {
    match state.db.list_users().await {
        Ok(users) => {
            let users: Vec<User> = users.into_iter().map(User::from).collect();
            HtmlTemplate(IndexTemplate { users }).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list users: {}", e);
            HtmlTemplate(NotImplementedTemplate {
                feature: "Error".to_string(),
                description: format!("Failed to list users: {}", e),
                back_url: "/".to_string(),
            })
            .into_response()
        }
    }
}

async fn create_user(
    State(state): State<GithubAppState>,
    Json(req): Json<CreateUserRequest>,
) -> impl IntoResponse {
    match state
        .db
        .create_user(&req.username, req.email.as_deref())
        .await
    {
        Ok(id) => {
            // If a GitHub token was provided, store it immediately.
            if let Some(token) = &req.github_token {
                if let Err(e) = state.db.update_user_github_token(id, Some(token)).await {
                    tracing::warn!(
                        user_id = id,
                        username = req.username,
                        error = %e,
                        "Failed to store GitHub token during user creation"
                    );
                }
            }
            tracing::info!(user_id = id, username = req.username, "User created");
            (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create user: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create user: {}", e),
            )
                .into_response()
        }
    }
}

/// POST /{username}/github-token
///
/// Update the GitHub Personal Access Token for a user.
/// This token is used for DORA metrics data fetching on behalf of the user.
async fn update_github_token(
    State(state): State<GithubAppState>,
    Path(username): Path<String>,
    Json(req): Json<UpdateGithubTokenRequest>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                format!("User '{}' not found", username),
            )
                .into_response();
        }
    };

    match state
        .db
        .update_user_github_token(user.id, Some(&req.token))
        .await
    {
        Ok(_) => {
            tracing::info!(username = %username, "GitHub token updated");
            (StatusCode::OK, "GitHub token updated").into_response()
        }
        Err(e) => {
            tracing::error!(username = %username, error = %e, "Failed to update GitHub token");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update GitHub token: {}", e),
            )
                .into_response()
        }
    }
}

/// GET /{username}/auth/github
///
/// Redirect the user to GitHub's OAuth authorization page.
/// The `state` parameter encodes the username (base64) and a random nonce
/// for CSRF protection. The nonce is stored in-memory so the callback can
/// verify it later.
async fn auth_github(
    State(state): State<GithubAppState>,
    Path(username): Path<String>,
) -> impl IntoResponse {
    let client_id = &state.config.github_oauth_client_id;
    let redirect_uri = &state.config.github_oauth_redirect_url;

    if client_id.is_empty() || redirect_uri.is_empty() {
        tracing::error!("GitHub OAuth is not configured: missing client_id or redirect_uri");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "GitHub OAuth is not configured on this server".to_string(),
        )
            .into_response();
    }

    // Generate a random 16-byte nonce as a hex string
    let mut nonce_bytes = [0u8; 16];
    rand::thread_rng().fill(&mut nonce_bytes);
    let nonce: String = nonce_bytes.iter().map(|b| format!("{:02x}", b)).collect();

    // Store nonce -> (username, created_at) so the callback can verify the CSRF token
    // Nonces expire after 10 minutes.
    {
        let mut nonces = state.oauth_nonces.lock().expect("nonce lock poisoned");
        nonces.insert(nonce.clone(), (username.clone(), Instant::now()));
    }

    // Build state = base64(username):nonce
    let username_b64 = general_purpose::STANDARD.encode(username.as_bytes());
    let state_param = format!("{}:{}", username_b64, nonce);

    let redirect_url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&state={}&scope=repo",
        client_id, redirect_uri, state_param,
    );

    tracing::info!(
        username = %username,
        "Redirecting to GitHub OAuth authorization"
    );

    (StatusCode::FOUND, Redirect::to(&redirect_url)).into_response()
}

/// OAuth error page with troubleshooting tips.
#[derive(Template)]
#[template(path = "oauth_error.html")]
struct OAuthErrorTemplate {
    error_title: String,
    error_message: String,
    troubleshooting_tips: Vec<String>,
    back_url: String,
}

/// GET /auth/github/callback
///
/// Handle the OAuth callback from GitHub:
/// 1. Parse `code` and `state` from query params
/// 2. Decode the state (base64(username):nonce)
/// 3. Verify the CSRF nonce (must exist and not be expired — 10 min TTL)
/// 4. Exchange the code for an access token
/// 5. Fetch the GitHub login from GET /user
/// 6. Store both token and login in the database
/// 7. Clean up the consumed nonce
/// 8. Redirect to the user profile
async fn auth_github_callback(
    State(state): State<GithubAppState>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    // ── Check for user denial ────────────────────────────────────────────
    if let Some(error) = params.get("error") {
        tracing::warn!("GitHub OAuth error returned: {}", error);
        return HtmlTemplate(OAuthErrorTemplate {
            error_title: "Access Denied".to_string(),
            error_message: format!(
                "You denied the GitHub authorization request.{}",
                params
                    .get("error_description")
                    .map(|d| format!(" GitHub says: {}", d))
                    .unwrap_or_default()
            ),
            troubleshooting_tips: vec![
                "Click the \"Connect to GitHub\" button on your profile page to try again."
                    .to_string(),
                "Make sure you grant the requested permissions when prompted.".to_string(),
                "If the problem persists, check that you are logged into the correct GitHub \
                 account."
                    .to_string(),
            ],
            back_url: "/".to_string(),
        })
        .into_response();
    }

    // ── Extract code ─────────────────────────────────────────────────────
    let code = match params.get("code") {
        Some(code) => code.clone(),
        None => {
            return HtmlTemplate(OAuthErrorTemplate {
                error_title: "Invalid Request".to_string(),
                error_message: "Missing authorization code from GitHub. The OAuth flow could \
                                not be completed."
                    .to_string(),
                troubleshooting_tips: vec![
                    "Start the OAuth flow again from your profile page.".to_string(),
                    "Ensure your browser allows cookies and redirects.".to_string(),
                ],
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    // ── Extract state ────────────────────────────────────────────────────
    let state_param = match params.get("state") {
        Some(s) => s.clone(),
        None => {
            return HtmlTemplate(OAuthErrorTemplate {
                error_title: "Invalid Request".to_string(),
                error_message: "Missing state parameter. This could indicate a CSRF attack or a \
                                broken OAuth flow."
                    .to_string(),
                troubleshooting_tips: vec![
                    "Start the OAuth flow again from your profile page.".to_string(),
                    "Do not manually craft or modify OAuth URLs.".to_string(),
                    "If this keeps happening, your session may have expired.".to_string(),
                ],
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    // ── Decode state = base64(username):nonce ────────────────────────────
    let (username_b64, nonce) = match state_param.split_once(':') {
        Some((b64, n)) => (b64, n.to_string()),
        None => {
            return HtmlTemplate(OAuthErrorTemplate {
                error_title: "Invalid Request".to_string(),
                error_message: "Malformed state parameter. The OAuth flow cannot be verified."
                    .to_string(),
                troubleshooting_tips: vec![
                    "Start the OAuth flow again from your profile page.".to_string(),
                    "This may be caused by URL decoding issues.".to_string(),
                ],
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let username_bytes = match general_purpose::STANDARD.decode(username_b64) {
        Ok(bytes) => bytes,
        Err(_) => {
            return HtmlTemplate(OAuthErrorTemplate {
                error_title: "Invalid Request".to_string(),
                error_message: "Could not decode the state parameter. The OAuth flow cannot be \
                                verified."
                    .to_string(),
                troubleshooting_tips: vec![
                    "Start the OAuth flow again from your profile page.".to_string(),
                    "This may be caused by a corrupted OAuth state.".to_string(),
                ],
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let username = match String::from_utf8(username_bytes) {
        Ok(u) => u,
        Err(_) => {
            return HtmlTemplate(OAuthErrorTemplate {
                error_title: "Invalid Request".to_string(),
                error_message: "Invalid username encoding in state parameter.".to_string(),
                troubleshooting_tips: vec![
                    "Start the OAuth flow again from your profile page.".to_string(),
                ],
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    // ── Verify nonce (consume it on any outcome) ─────────────────────────
    let nonce_valid = {
        let mut nonces = state.oauth_nonces.lock().expect("nonce lock poisoned");
        match nonces.remove(&nonce) {
            Some((stored_username, created_at)) if stored_username == username => {
                if created_at.elapsed() > Duration::from_secs(600) {
                    tracing::warn!(%username, "Expired OAuth nonce (TTL 10 min)");
                    None // expired
                } else {
                    Some(true) // valid
                }
            }
            Some((_stored_username, _created_at)) => {
                tracing::warn!(%username, "OAuth nonce username mismatch — possible CSRF");
                Some(false) // username mismatch
            }
            None => {
                tracing::warn!(%username, "OAuth nonce not found — possible replay or expired");
                None // not found
            }
        }
    };

    match nonce_valid {
        None => {
            // Nonce was not found or expired — we already removed it above
            return HtmlTemplate(OAuthErrorTemplate {
                error_title: "Request Expired".to_string(),
                error_message: "This authorization request has expired or the security token \
                                was already used. OAuth requests must be completed within 10 \
                                minutes."
                    .to_string(),
                troubleshooting_tips: vec![
                    "Go back to your profile page and click \"Connect to GitHub\" again."
                        .to_string(),
                    "Complete the GitHub authorization promptly.".to_string(),
                    "Each authorization request can only be used once.".to_string(),
                ],
                back_url: "/".to_string(),
            })
            .into_response();
        }
        Some(false) => {
            // Username mismatch — possible CSRF
            return HtmlTemplate(OAuthErrorTemplate {
                error_title: "Invalid Request".to_string(),
                error_message: "The OAuth state does not match the current user session. This \
                                could be a security issue."
                    .to_string(),
                troubleshooting_tips: vec![
                    "Start the OAuth flow again from your profile page.".to_string(),
                    "Do not share OAuth authorization URLs.".to_string(),
                    "If you see this repeatedly, your session may have been tampered with."
                        .to_string(),
                ],
                back_url: "/".to_string(),
            })
            .into_response();
        }
        Some(true) => {
            // Nonce is valid — continue
            tracing::info!(%username, "OAuth nonce verified successfully");
        }
    }

    // ── Exchange code for access token ───────────────────────────────────
    let client = reqwest::Client::new();
    let token_response = match client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "client_id": state.config.github_oauth_client_id,
            "client_secret": state.config.github_oauth_client_secret,
            "code": code,
            "redirect_uri": state.config.github_oauth_redirect_url,
        }))
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!(%username, error = %e, "Failed to reach GitHub token endpoint");
            return HtmlTemplate(OAuthErrorTemplate {
                error_title: "GitHub Error".to_string(),
                error_message: format!(
                    "Could not reach GitHub to exchange the authorization code: {}",
                    e
                ),
                troubleshooting_tips: vec![
                    "Check your internet connection.".to_string(),
                    "Verify that github.com is accessible.".to_string(),
                    "Try again — this may be a transient network issue.".to_string(),
                ],
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let token_data: GithubAccessTokenResponse = match token_response.json().await {
        Ok(data) => data,
        Err(e) => {
            tracing::error!(%username, error = %e, "Failed to parse token response from GitHub");
            return HtmlTemplate(OAuthErrorTemplate {
                error_title: "GitHub Error".to_string(),
                error_message: "Received an unexpected response from GitHub during token \
                                exchange."
                    .to_string(),
                troubleshooting_tips: vec![
                    "Try again later.".to_string(),
                    "Check the server logs for more details.".to_string(),
                ],
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let access_token = match token_data.access_token {
        Some(token) => token,
        None => {
            let error_desc = token_data
                .error
                .unwrap_or_else(|| "unknown_error".to_string());
            tracing::error!(
                %username,
                error = %error_desc,
                "GitHub token exchange returned an error"
            );
            return HtmlTemplate(OAuthErrorTemplate {
                error_title: "GitHub Error".to_string(),
                error_message: format!(
                    "GitHub returned an error during token exchange: {}",
                    token_data
                        .error_description
                        .as_deref()
                        .unwrap_or("No details provided")
                ),
                troubleshooting_tips: vec![
                    "Verify that the GitHub OAuth App is correctly configured.".to_string(),
                    "Check that the client ID and client secret are correct.".to_string(),
                    "Ensure the redirect URL matches what is registered in the GitHub OAuth App."
                        .to_string(),
                ],
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    // ── Fetch GitHub user info ───────────────────────────────────────────
    let user_response = match client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "repohub")
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!(%username, error = %e, "Failed to fetch GitHub user info");
            return HtmlTemplate(OAuthErrorTemplate {
                error_title: "GitHub Error".to_string(),
                error_message: format!("Could not fetch your GitHub profile information: {}", e),
                troubleshooting_tips: vec![
                    "Check your internet connection.".to_string(),
                    "The token was obtained but user info fetch failed. Try again.".to_string(),
                ],
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let github_login = match user_response.json::<GithubUserResponse>().await {
        Ok(data) => data.login,
        Err(e) => {
            tracing::error!(%username, error = %e, "Failed to parse GitHub user response");
            return HtmlTemplate(OAuthErrorTemplate {
                error_title: "GitHub Error".to_string(),
                error_message: "Received an unexpected response from the GitHub User API."
                    .to_string(),
                troubleshooting_tips: vec![
                    "Try again later.".to_string(),
                    "Check the server logs for more details.".to_string(),
                ],
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    // ── Look up the user in our database ─────────────────────────────────
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(e) => {
            tracing::error!(%username, error = %e, "User not found during OAuth callback");
            return HtmlTemplate(OAuthErrorTemplate {
                error_title: "Invalid Request".to_string(),
                error_message: format!(
                    "User '{}' not found in the system. The OAuth flow may have been started \
                     for a deleted user.",
                    username
                ),
                troubleshooting_tips: vec![
                    "Create a new user account first.".to_string(),
                    "Then connect to GitHub from your profile page.".to_string(),
                ],
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    // ── Store token and GitHub login ─────────────────────────────────────
    if let Err(e) = state
        .db
        .update_user_github_token(user.id, Some(&access_token))
        .await
    {
        tracing::error!(%username, error = %e, "Failed to store GitHub token");
        return HtmlTemplate(OAuthErrorTemplate {
            error_title: "Server Error".to_string(),
            error_message: "Failed to store your GitHub credentials. Please try again.".to_string(),
            troubleshooting_tips: vec![
                "Try again.".to_string(),
                "If the problem persists, contact the server administrator.".to_string(),
            ],
            back_url: "/".to_string(),
        })
        .into_response();
    }

    if let Err(e) = state
        .db
        .update_user_github_login(user.id, Some(&github_login))
        .await
    {
        tracing::error!(%username, error = %e, "Failed to store GitHub login");
        // Non-fatal — token is stored, login can be retried
    }

    tracing::info!(
        %username,
        github_login = %github_login,
        "GitHub OAuth flow completed successfully"
    );

    (StatusCode::FOUND, Redirect::to(&format!("/{}", username))).into_response()
}

async fn user(
    State(state): State<GithubAppState>,
    Path(username): Path<String>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => User::from(user),
        Err(e) => {
            tracing::error!("Failed to get user '{}': {}", username, e);
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    match state.db.list_projects_by_owner(user.id).await {
        Ok(projects) => {
            let projects: Vec<Project> = projects.into_iter().map(Project::from).collect();
            HtmlTemplate(UserTemplate { user, projects }).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list projects for user '{}': {}", username, e);
            HtmlTemplate(NotImplementedTemplate {
                feature: "Error".to_string(),
                description: format!("Failed to list projects: {}", e),
                back_url: "/".to_string(),
            })
            .into_response()
        }
    }
}

async fn create_project(
    State(state): State<GithubAppState>,
    Path(username): Path<String>,
    Json(req): Json<CreateProjectRequest>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("User not found: {}", e)).into_response();
        }
    };

    match state
        .db
        .create_project(&req.name, user.id, req.description.as_deref())
        .await
    {
        Ok(id) => {
            tracing::info!(
                project_id = id,
                owner = username,
                project_name = req.name,
                "Project created"
            );
            (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create project: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create project: {}", e),
            )
                .into_response()
        }
    }
}

async fn project(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(error) => {
            tracing::error!(username, project_name, %error, "Failed to get user");
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => Project::from(project),
        Err(error) => {
            tracing::error!(username, project_name, %error, "Failed to get project");
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    match state.db.list_repositories_by_project(project.id).await {
        Ok(repos) => {
            let repositories: Vec<Repository> = repos.into_iter().map(Repository::from).collect();
            HtmlTemplate(ProjectTemplate {
                username,
                project,
                repositories,
            })
            .into_response()
        }
        Err(error) => {
            tracing::error!(username, project_name, %error, "Failed to list repositories");
            HtmlTemplate(NotImplementedTemplate {
                feature: "Error".to_string(),
                description: format!("Failed to list repositories: {}", error),
                back_url: format!("/{}", username),
            })
            .into_response()
        }
    }
}

async fn create_repository(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
    Json(req): Json<CreateRepositoryRequest>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("User not found: {}", e)).into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("Project not found: {}", e)).into_response();
        }
    };

    let git_url = match state.repo_service.create_or_clone_repository(
        &username,
        &req.name,
        req.git_url.as_deref(),
    ) {
        Ok(url) => url,
        Err(e) => {
            tracing::error!(error = ?e, "Failed to create/clone repository");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create repository: {}", e),
            )
                .into_response();
        }
    };

    match state
        .db
        .create_repository(project.id, &req.name, &git_url)
        .await
    {
        Ok(id) => {
            tracing::info!(
                repo_id = id,
                project = project_name,
                repo_name = req.name,
                "Repository created"
            );
            (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create repository: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create repository: {}", e),
            )
                .into_response()
        }
    }
}

async fn configuration(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    let repositories = match state.db.list_repositories_by_project(project.id).await {
        Ok(repos) => repos.into_iter().map(Repository::from).collect::<Vec<_>>(),
        Err(_error) => Vec::new(),
    };

    let repositories_json =
        serde_json::to_string(&repositories).unwrap_or_else(|_| "[]".to_string());

    HtmlTemplate(ConfigurationTemplate {
        username,
        project_name,
        repositories: repositories.clone(),
        repositories_json,
    })
    .into_response()
}

async fn save_configuration(
    State(_state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
    Json(req): Json<SaveConfigurationRequest>,
) -> impl IntoResponse {
    tracing::info!(
        username = username,
        project = project_name,
        "Saving project configuration"
    );

    tracing::debug!(config = ?req.configuration, "Received configuration");

    (StatusCode::OK, Json(serde_json::json!({ "success": true }))).into_response()
}

async fn testing(
    State(_state): State<GithubAppState>,
    Path((username, project)): Path<(String, String)>,
) -> impl IntoResponse {
    HtmlTemplate(NotImplementedTemplate {
		feature: "E2E Testing & Monitoring".to_string(),
		description: "Define and run end-to-end tests across all services in this project. Monitor performance, measure service health, and validate integration points.".to_string(),
		back_url: format!("/{}/{}", username, project),
	})
}

async fn agents(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    let agents = vec![
        AgentView {
            name: "Build Orchestrator".to_string(),
            status: "Healthy".to_string(),
            description: "Schedules CI builds and fan-out jobs".to_string(),
            last_run: "5 min ago".to_string(),
            next_run: "Every 10 min".to_string(),
        },
        AgentView {
            name: "Dependency Watcher".to_string(),
            status: "Paused".to_string(),
            description: "Tracks upstream updates across repos".to_string(),
            last_run: "Yesterday".to_string(),
            next_run: "Manual".to_string(),
        },
        AgentView {
            name: "Release Assistant".to_string(),
            status: "Healthy".to_string(),
            description: "Prepares release notes and changelog".to_string(),
            last_run: "2 hours ago".to_string(),
            next_run: "Daily".to_string(),
        },
    ];

    let runs = vec![
        RunView {
            name: "Build Orchestrator".to_string(),
            status: "Success".to_string(),
            duration: "2m 14s".to_string(),
            started_at: "2026-05-05 09:12".to_string(),
        },
        RunView {
            name: "Release Assistant".to_string(),
            status: "Success".to_string(),
            duration: "54s".to_string(),
            started_at: "2026-05-05 07:02".to_string(),
        },
        RunView {
            name: "Dependency Watcher".to_string(),
            status: "Skipped".to_string(),
            duration: "--".to_string(),
            started_at: "2026-05-04 20:20".to_string(),
        },
    ];

    HtmlTemplate(AgentsTemplate {
        username,
        project_name: project.name,
        agents,
        runs,
    })
    .into_response()
}

async fn documentation(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    let sources = vec![
        DocSourceView {
            name: "Main Docs".to_string(),
            location: "repo/docs".to_string(),
            updated_at: "2026-05-05".to_string(),
            status: "Healthy".to_string(),
        },
        DocSourceView {
            name: "API Reference".to_string(),
            location: "repo/openapi".to_string(),
            updated_at: "2026-05-04".to_string(),
            status: "Syncing".to_string(),
        },
        DocSourceView {
            name: "Runbooks".to_string(),
            location: "repo/runbooks".to_string(),
            updated_at: "2026-05-01".to_string(),
            status: "Healthy".to_string(),
        },
    ];

    let documents = vec![
        DocumentView {
            title: "Architecture Overview".to_string(),
            repo: "core-services".to_string(),
            updated_at: "2026-05-05".to_string(),
            status: "Published".to_string(),
        },
        DocumentView {
            title: "CI Workflow".to_string(),
            repo: "ci-pipelines".to_string(),
            updated_at: "2026-05-04".to_string(),
            status: "Draft".to_string(),
        },
        DocumentView {
            title: "Ops Runbook".to_string(),
            repo: "runbooks".to_string(),
            updated_at: "2026-05-01".to_string(),
            status: "Published".to_string(),
        },
    ];

    HtmlTemplate(DocumentationTemplate {
        username,
        project_name: project.name,
        sources,
        documents,
    })
    .into_response()
}

async fn stats(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    let kpis = vec![
        KpiView {
            label: "Build Success".to_string(),
            value: "93%".to_string(),
        },
        KpiView {
            label: "Avg Build Time".to_string(),
            value: "6m 42s".to_string(),
        },
        KpiView {
            label: "Queued Jobs".to_string(),
            value: "4".to_string(),
        },
        KpiView {
            label: "Active Repos".to_string(),
            value: "12".to_string(),
        },
    ];

    let build_series = vec![
        ChartPoint {
            label: "Mon".to_string(),
            value: 12,
            width_pct: 55,
        },
        ChartPoint {
            label: "Tue".to_string(),
            value: 18,
            width_pct: 85,
        },
        ChartPoint {
            label: "Wed".to_string(),
            value: 15,
            width_pct: 70,
        },
        ChartPoint {
            label: "Thu".to_string(),
            value: 21,
            width_pct: 100,
        },
        ChartPoint {
            label: "Fri".to_string(),
            value: 16,
            width_pct: 75,
        },
    ];

    let failure_series = vec![
        ChartPoint {
            label: "Infra".to_string(),
            value: 4,
            width_pct: 60,
        },
        ChartPoint {
            label: "Tests".to_string(),
            value: 2,
            width_pct: 30,
        },
        ChartPoint {
            label: "Deps".to_string(),
            value: 1,
            width_pct: 15,
        },
        ChartPoint {
            label: "Timeout".to_string(),
            value: 3,
            width_pct: 45,
        },
    ];

    let recent_runs = vec![
        RunView {
            name: "backend-service".to_string(),
            status: "Success".to_string(),
            duration: "5m 02s".to_string(),
            started_at: "2026-05-05 08:50".to_string(),
        },
        RunView {
            name: "worker".to_string(),
            status: "Failed".to_string(),
            duration: "7m 45s".to_string(),
            started_at: "2026-05-05 07:30".to_string(),
        },
        RunView {
            name: "frontend".to_string(),
            status: "Success".to_string(),
            duration: "4m 18s".to_string(),
            started_at: "2026-05-05 07:05".to_string(),
        },
    ];

    let monitors = vec![
        MonitorView {
            name: "Public API".to_string(),
            target: "https://api.repohub.local/health".to_string(),
            status: "Up".to_string(),
            latency_ms: 182,
            uptime: "99.98%".to_string(),
            last_check: "45s ago".to_string(),
            ssl_expires: "89 days".to_string(),
            regions: "6 regions".to_string(),
            alerting: "Slack + Email".to_string(),
        },
        MonitorView {
            name: "Docs Site".to_string(),
            target: "https://docs.repohub.local".to_string(),
            status: "Degraded".to_string(),
            latency_ms: 840,
            uptime: "99.62%".to_string(),
            last_check: "2m ago".to_string(),
            ssl_expires: "41 days".to_string(),
            regions: "4 regions".to_string(),
            alerting: "PagerDuty".to_string(),
        },
        MonitorView {
            name: "Internal CI".to_string(),
            target: "https://ci.repohub.local/".to_string(),
            status: "Down".to_string(),
            latency_ms: 0,
            uptime: "98.01%".to_string(),
            last_check: "now".to_string(),
            ssl_expires: "15 days".to_string(),
            regions: "3 regions".to_string(),
            alerting: "Slack".to_string(),
        },
    ];

    let monitor_options = vec![
        MonitorOptionView {
            name: "HTTP/HTTPS Checks".to_string(),
            description: "Status code, response time, and content validation".to_string(),
        },
        MonitorOptionView {
            name: "SSL Monitoring".to_string(),
            description: "Certificate expiry and chain validation alerts".to_string(),
        },
        MonitorOptionView {
            name: "Multi-Region Probes".to_string(),
            description: "Run checks from multiple regions for latency insights".to_string(),
        },
        MonitorOptionView {
            name: "Alerting Policies".to_string(),
            description: "Slack, email, PagerDuty, and webhook notifications".to_string(),
        },
        MonitorOptionView {
            name: "Status Pages".to_string(),
            description: "Public or private status visibility for stakeholders".to_string(),
        },
        MonitorOptionView {
            name: "Latency & Uptime SLOs".to_string(),
            description: "Targets and historical uptime reporting".to_string(),
        },
    ];

    HtmlTemplate(StatsTemplate {
        username,
        project_name: project.name,
        kpis,
        build_series,
        failure_series,
        recent_runs,
        monitors,
        monitor_options,
    })
    .into_response()
}

async fn milestones(
    State(state): State<GithubAppState>,
    Path((username, project_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    let milestones = vec![
        MilestoneView {
            title: "CI Stabilization".to_string(),
            due_date: "2026-05-18".to_string(),
            status: "In Progress".to_string(),
            progress: 65,
            description: "Reduce flaky tests and improve build times".to_string(),
        },
        MilestoneView {
            title: "Docs Revamp".to_string(),
            due_date: "2026-05-25".to_string(),
            status: "Planned".to_string(),
            progress: 20,
            description: "Consolidate runbooks and API reference".to_string(),
        },
        MilestoneView {
            title: "Release 2.3".to_string(),
            due_date: "2026-06-05".to_string(),
            status: "At Risk".to_string(),
            progress: 45,
            description: "Finalize features and prep release notes".to_string(),
        },
    ];

    HtmlTemplate(MilestonesTemplate {
        username,
        project_name: project.name,
        milestones,
    })
    .into_response()
}

async fn repo_flake(
    State(state): State<GithubAppState>,
    Path((username, project_name, repo_name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let flake_metadata = state
        .repo_service
        .parse_flake_metadata(&username, &repo_name);

    HtmlTemplate(FlakeTemplate {
        username,
        project_name,
        repo_name,
        flake_metadata,
    })
}

async fn builds(
    State(_state): State<GithubAppState>,
    Path((username, project, repo, _id)): Path<(String, String, String, i64)>,
) -> impl IntoResponse {
    HtmlTemplate(NotImplementedTemplate {
		feature: "Build Details".to_string(),
		description: "View build logs, status, and artifacts. This will integrate with the CI service to display build information.".to_string(),
		back_url: format!("/{}/{}/{}", username, project, repo),
	})
}

async fn repo(
    State(state): State<GithubAppState>,
    Path((username, project_name, repo_name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "User Not Found".to_string(),
                description: format!("User '{}' not found", username),
                back_url: "/".to_string(),
            })
            .into_response();
        }
    };

    let project = match state.db.get_project(user.id, &project_name).await {
        Ok(project) => project,
        Err(_error) => {
            return HtmlTemplate(NotImplementedTemplate {
                feature: "Project Not Found".to_string(),
                description: format!("Project '{}' not found", project_name),
                back_url: format!("/{}", username),
            })
            .into_response();
        }
    };

    match state.db.get_repository(project.id, &repo_name).await {
        Ok(repo) => {
            let repo = Repository::from(repo);
            HtmlTemplate(RepositoryTemplate {
                username,
                project_name,
                repo,
            })
            .into_response()
        }
        Err(_error) => {
            tracing::error!("Failed to get repository: {}", repo_name);
            HtmlTemplate(NotImplementedTemplate {
                feature: "Repository Not Found".to_string(),
                description: format!("Repository '{}' not found", repo_name),
                back_url: format!("/{}/{}", username, project_name),
            })
            .into_response()
        }
    }
}

/// GET /{username}/github/status
///
/// Return the GitHub connection status for a user:
/// - `{ "connected": true, "github_login": "..." }` when token is stored
/// - `{ "connected": false }` when no token
async fn github_status(
    State(state): State<GithubAppState>,
    Path(username): Path<String>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                format!("User '{}' not found", username),
            )
                .into_response();
        }
    };

    if user.github_token.is_some() {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "connected": true,
                "github_login": user.github_login,
            })),
        )
            .into_response()
    } else {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "connected": false,
            })),
        )
            .into_response()
    }
}

/// DELETE /{username}/github-token
///
/// Disconnect GitHub by clearing the stored token and GitHub login.
/// Returns 200 on success.
async fn disconnect_github(
    State(state): State<GithubAppState>,
    Path(username): Path<String>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                format!("User '{}' not found", username),
            )
                .into_response();
        }
    };

    if let Err(e) = state.db.update_user_github_token(user.id, None).await {
        tracing::error!(%username, error = %e, "Failed to clear GitHub token");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to disconnect GitHub: {}", e),
        )
            .into_response();
    }

    if let Err(e) = state.db.update_user_github_login(user.id, None).await {
        tracing::error!(%username, error = %e, "Failed to clear GitHub login");
        // Non-fatal — token is already cleared
    }

    tracing::info!(%username, "GitHub disconnected");
    (StatusCode::OK, "GitHub disconnected").into_response()
}

/// GET /{username}/github/repos
///
/// Fetch the user's GitHub repositories through the shared auth abstraction.
/// The underlying credential can be a PAT today or an installation token once
/// the migration is fully wired.
///
/// Returns 400 if the user has no GitHub token configured.
async fn github_repos(
    State(state): State<GithubAppState>,
    Path(username): Path<String>,
) -> impl IntoResponse {
    let user = match state.db.get_user_by_username(&username).await {
        Ok(user) => user,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                format!("User '{}' not found", username),
            )
                .into_response();
        }
    };

    let token = match &user.github_token {
        Some(t) if !t.is_empty() => t.clone(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "No GitHub token configured for this user. Use the OAuth flow to connect your GitHub account.".to_string(),
            )
                .into_response();
        }
    };

    let auth = crate::adapters::github::auth::GithubAuth::from_pat(token);

    let repos = match auth.list_authenticated_user_repositories().await {
        Ok(repos) => repos,
        Err(e) => {
            tracing::error!(%username, error = %e, "Failed to fetch GitHub repos");
            return (
                StatusCode::BAD_GATEWAY,
                format!("Failed to fetch GitHub repos: {}", e),
            )
                .into_response();
        }
    };

    tracing::info!(
        %username,
        repo_count = repos.len(),
        "Fetched GitHub repos"
    );

    (StatusCode::OK, Json(repos)).into_response()
}

pub fn routes() -> Router<GithubAppState> {
    Router::new()
        .route("/", get(index))
        .route("/users", post(create_user))
        .route("/{username}", get(user))
        .route(
            "/{username}/github-token",
            post(update_github_token).delete(disconnect_github),
        )
        .route("/{username}/github/status", get(github_status))
        .route("/{username}/github/repos", get(github_repos))
        .route("/{username}/auth/github", get(auth_github))
        .route("/auth/github/callback", get(auth_github_callback))
        .route("/{username}/projects", post(create_project))
        .route("/{username}/{project}", get(project))
        .route(
            "/{username}/{project}/repositories",
            post(create_repository),
        )
        .route("/{username}/{project}/testing", get(testing))
        .route(
            "/{username}/{project}/configuration",
            get(configuration).post(save_configuration),
        )
        .route("/{username}/{project}/agents", get(agents))
        .route("/{username}/{project}/documentation", get(documentation))
        .route("/{username}/{project}/stats", get(stats))
        .route("/{username}/{project}/milestones", get(milestones))
        .route("/{username}/{project}/{repo}", get(repo))
        .route("/{username}/{project}/{repo}/builds/{id}", get(builds))
        .route("/{username}/{project}/{repo}/flake", get(repo_flake))
}
