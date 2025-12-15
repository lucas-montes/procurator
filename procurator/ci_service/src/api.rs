//! CI API Handlers
//!
//! This module provides HTTP API endpoints for the CI service:
//! - `POST /builds` - Create a new build (called by Git post-receive hooks)
//! - `GET /builds` - List all builds
//! - `GET /builds/:id` - Get specific build
//! - `GET /builds/:id/logs` - Get build logs
//! - `GET /repos` - List repositories
//! - `POST /repos` - Create repository
//! - `GET /repos/:name` - Get repository details
//! - `GET /events` - Server-Sent Events for real-time updates

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Sse,
    routing::{get, post},
    Json, Router,
};
use futures::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::path::PathBuf;
use std::time::Duration;
use tracing::info;

use crate::config::Config;
use crate::domain::{Build, BuildStatus};
use crate::nix_parser::FlakeMetadata;
use crate::AppState;

#[derive(Debug, Serialize)]
pub struct BuildInfo {
    id: i64,
    repo: String,
    commit_hash: String,
    commit_short: String,
    branch: String,
    status: BuildStatus,
    retry_count: i64,
    max_retries: i64,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    duration_seconds: Option<i64>,
}

impl From<Build> for BuildInfo {
    fn from(build: Build) -> Self {
        let commit_hash = build.commit_hash();
        let commit_short = if commit_hash.len() >= 8 {
            commit_hash[..8].to_string()
        } else {
            commit_hash.to_string()
        };

        let duration_seconds =
            if let (Some(started), Some(finished)) = (build.started_at(), build.finished_at()) {
                chrono::NaiveDateTime::parse_from_str(finished, "%Y-%m-%d %H:%M:%S")
                    .ok()
                    .and_then(|f| {
                        chrono::NaiveDateTime::parse_from_str(started, "%Y-%m-%d %H:%M:%S")
                            .ok()
                            .map(|s| (f - s).num_seconds())
                    })
            } else {
                None
            };

        BuildInfo {
            id: build.id(),
            repo: build.repo_name().to_string(),
            commit_hash: commit_hash.to_string(),
            commit_short,
            branch: build.branch().to_string(),
            status: build.status(),
            retry_count: build.retry_count(),
            max_retries: build.max_retries(),
            created_at: build.created_at().to_string(),
            started_at: build.started_at().map(|s| s.to_string()),
            finished_at: build.finished_at().map(|s| s.to_string()),
            duration_seconds,
        }
    }
}

#[derive(Debug, Serialize)]
struct BuildsListResponse {
    builds: Vec<BuildInfo>,
    total: usize,
}

#[derive(Debug, Serialize)]
struct RepoInfo {
    name: String,
    path: PathBuf,
    builds_count: i64,
    last_build: Option<BuildInfo>,
}

#[derive(Debug, Serialize)]
struct RepoDetails {
    name: String,
    git_url: String,
    builds_count: i64,
    recent_builds: Vec<BuildInfo>,
    setup_instructions: SetupInstructions,
    flake_metadata: Option<FlakeMetadata>,
}

#[derive(Debug, Serialize)]
struct SetupInstructions {
    new_repo: Vec<String>,
    existing_repo: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum BuildEvent {
    Created { build: BuildInfo },
    Updated { build: BuildInfo },
    Completed { build: BuildInfo },
}

#[derive(Debug, Deserialize)]
pub struct CreateRepoRequest {
    name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateRepoResponse {
    name: String,
    path: String,
    git_url: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct BuildRequest {
    repo: String,
    bare_repo_path: String,
    old_rev: Option<String>,
    new_rev: String,
    #[serde(rename = "ref")]
    ref_name: String,
    commit_author: Option<String>,
    commit_email: Option<String>,
    commit_message: Option<String>,
    gpg_status: Option<String>,
    gpg_key: Option<String>,
    gpg_signer: Option<String>,
    pusher: Option<String>,
    ssh_client_ip: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BuildResponse {
    id: i64,
    status: BuildStatus,
}

async fn create_build(
    State(state): State<AppState>,
    Json(req): Json<BuildRequest>,
) -> impl axum::response::IntoResponse {
    info!(?req, "Build request received");

    // Extract branch name
    let branch = req
        .ref_name
        .strip_prefix("refs/heads/")
        .unwrap_or(&req.ref_name);

    // Convert bare_repo_path to PathBuf
    let repo_path = PathBuf::from(&req.bare_repo_path);

    //TODO: check if this works as expected

    // Enqueue build with bare repo path
    match state.queue.enqueue(repo_path, &req.new_rev, branch).await {
        Ok(id) => {
            info!(
                build_id = id,
                repo = req.repo.as_str(),
                branch = branch,
                "Build enqueued successfully"
            );
            Ok((
                StatusCode::ACCEPTED,
                Json(BuildResponse {
                    id,
                    status: BuildStatus::Queued,
                }),
            ))
        }
        Err(e) => {
            tracing::error!(
                repo = req.repo.as_str(),
                branch = branch,
                error = %e,
                "Failed to enqueue build"
            );
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to enqueue build: {}", e),
            ))
        }
    }
}

/// List all builds
async fn list_builds(
    State(state): State<AppState>,
) -> Result<Json<BuildsListResponse>, (StatusCode, String)> {
    match state.queue.list_all_builds().await {
        Ok(builds) => {
            let total = builds.len();
            let builds_info = builds.into_iter().map(BuildInfo::from).collect();
            Ok(Json(BuildsListResponse {
                builds: builds_info,
                total,
            }))
        }
        Err(e) => {
            tracing::error!("Failed to list builds: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to list builds: {}", e),
            ))
        }
    }
}

/// Get a specific build by ID
async fn get_build(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<BuildInfo>, (StatusCode, String)> {
    match state.queue.get_build(id).await {
        Ok(build) => Ok(Json(BuildInfo::from(build))),
        Err(e) => {
            tracing::error!("Failed to get build {}: {}", id, e);
            Err((StatusCode::NOT_FOUND, format!("Build not found: {}", e)))
        }
    }
}

/// Get build logs
async fn get_build_logs(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    tracing::info!("Fetching logs for build #{}", id);

    match state.queue.get_build_logs(id).await {
        Ok(Some(logs)) => {
            tracing::info!("Returning {} bytes of logs for build #{}", logs.len(), id);
            Ok(Json(serde_json::json!({ "logs": logs })))
        }
        Ok(None) => {
            tracing::warn!("No logs found for build #{}", id);
            Ok(Json(serde_json::json!({ "logs": "No logs available yet" })))
        }
        Err(e) => {
            tracing::error!("Failed to get logs for build {}: {}", id, e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to get logs: {}", e) })),
            ))
        }
    }
}

/// List all repositories
async fn list_repos(
    State(state): State<AppState>,
) -> Result<Json<Vec<RepoInfo>>, (StatusCode, String)> {
    match state.git_manager.list_repos("lucas").await {
        Ok(repos) => {
            let mut repo_infos = Vec::new();

            for repo_path in repos {
                let name = repo_path.repo_name().to_string();

                let builds_from_db = state.repo_store.list_repos_with_build_counts().await.ok().unwrap_or_default();
                let builds_count = builds_from_db
                    .iter()
                    .find(|(db_name, _, _)| db_name == &name)
                    .map(|(_, _, count)| *count)
                    .unwrap_or(0);

                let last_build = state
                    .queue
                    .get_latest_build_for_repo(&name)
                    .await
                    .ok()
                    .flatten()
                    .map(BuildInfo::from);

                repo_infos.push(RepoInfo {
                    name,
                    path: repo_path.bare_repo_path(),
                    builds_count,
                    last_build,
                });
            }

            Ok(Json(repo_infos))
        }
        Err(e) => {
            tracing::error!("Failed to list repos: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to list repos: {}", e),
            ))
        }
    }
}

/// Create a new bare repository
async fn create_repo(
    State(state): State<AppState>,
    Json(req): Json<CreateRepoRequest>,
) -> Result<(StatusCode, Json<CreateRepoResponse>), (StatusCode, String)> {
    info!("Creating repo: {}", req.name);

    if !req
        .name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid repo name. Use only alphanumeric, dash, and underscore".to_string(),
        ));
    }

    let username = "lucas";
    let config = Config::init();

    match state
        .git_manager
        .create_bare_repo(username, &req.name)
        .await
    {
        Ok(repo_path) => {
            info!("Repository created at: {}", repo_path);

            Ok((
                StatusCode::CREATED,
                Json(CreateRepoResponse {
                    name: req.name.clone(),
                    path: repo_path.to_string(),
                    git_url: repo_path.to_ssh_url(&config.domain),
                }),
            ))
        }
        Err(e) => {
            tracing::error!("Failed to create repo: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create repo: {}", e),
            ))
        }
    }
}

/// Get details for a specific repository
async fn get_repo(
    State(state): State<AppState>,
    Path((username, repo)): Path<(String, String)>,
) -> Result<Json<RepoDetails>, (StatusCode, String)> {
    let repos = state
        .git_manager
        .list_repos(&username)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let repo_path = repos.iter().find(|r| r.repo_name() == repo).ok_or((
        StatusCode::NOT_FOUND,
        format!("Repository '{}' not found", repo),
    ))?;

    let all_builds = state
        .queue
        .list_all_builds()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let repo_builds: Vec<BuildInfo> = all_builds
        .into_iter()
        .filter(|b| b.repo_name() == repo)
        .take(10)
        .map(BuildInfo::from)
        .collect();

    let builds_count = repo_builds.len() as i64;

    let flake_metadata = match repo_path.try_into() {
        Ok(metadata) => {
            info!("Parsed flake metadata for repo: {}", repo);
            Some(metadata)
        }
        Err(e) => {
            info!("Could not parse flake metadata for {}: {}", repo, e);
            None
        }
    };

    let git_url = repo_path.to_ssh_url(&Config::init().domain);

    let details = RepoDetails {
        name: repo.clone(),
        git_url: git_url.clone(),
        builds_count,
        recent_builds: repo_builds,
        setup_instructions: SetupInstructions {
            new_repo: vec![
                format!("echo \"# {}\" >> README.md", repo),
                "git init".to_string(),
                "git add .".to_string(),
                "git commit -m \"Initial commit\"".to_string(),
                format!("git remote add origin {}", git_url),
                "git push -u origin main".to_string(),
            ],
            existing_repo: vec![
                format!("git remote add origin {}", git_url),
                "git push -u origin main".to_string(),
            ],
        },
        flake_metadata,
    };

    Ok(Json(details))
}

/// Stream build events in real-time
async fn build_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<axum::response::sse::Event, Infallible>>> {
    let stream = stream::unfold(state, |state| async move {
        tokio::time::sleep(Duration::from_secs(2)).await;

        let builds = state.queue.list_all_builds().await.ok()?;

        if let Some(latest) = builds.first() {
            let event = BuildEvent::Updated {
                build: BuildInfo::from(latest.clone()),
            };

            let data = serde_json::to_string(&event).ok()?;
            let sse_event = axum::response::sse::Event::default().data(data);

            Some((Ok(sse_event), state))
        } else {
            let sse_event = axum::response::sse::Event::default().comment("ping");
            Some((Ok(sse_event), state))
        }
    });

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("keep-alive"),
    )
}

/// Build the API routes
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/builds", post(create_build))
        .route("/builds", get(list_builds))
        .route("/builds/{id}", get(get_build))
        .route("/builds/{id}/logs", get(get_build_logs))
        .route("/events", get(build_events))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_info_from_build() {
        BuildRequest {
            repo: "dummy".into(),
            bare_repo_path: "/var/lib/git-server/lucas/dummy.git".into(),
            old_rev: Some("0000000000000000000000000000000000000000".into()),
            new_rev: "1e43a4529500115f72383235e7112e4e3ba91005".into(),
            ref_name: "refs/heads/master".into(),
            commit_author: Some("Lucas".into()),
            commit_email: Some("lluc23@hotmail.com".into()),
            commit_message: Some("4".into()),
            gpg_status: None,
            gpg_key: None,
            gpg_signer: None,
            pusher: Some("git".into()),
            ssh_client_ip: Some("192.168.1.15".into()),
        };
    }
}
