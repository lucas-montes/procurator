//! Web API Handlers
//!
//! Provides HTTP endpoints for the CI service's web interface and API:
//! - `GET /` - Single Page Application (SPA)
//! - `GET /api/builds` - List all builds with pagination
//! - `GET /api/builds/{id}` - Get build details
//! - `GET /api/builds/{id}/logs` - Stream build logs
//! - `GET /api/repos` - List repositories
//! - `POST /api/repos` - Create repository
//! - `GET /api/events` - Server-Sent Events (SSE) for real-time updates
//!
//! The web module serves both the UI and the REST API, with support for
//! real-time build status updates via SSE.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, Sse},
    Json,
};
use futures::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::time::Duration;
use tracing::info;

use crate::queue::{Build, BuildStatus};
use crate::AppState;

// ============================================================================
// Static HTML - Single Page Application
// ============================================================================

pub async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

// ============================================================================
// API Endpoints
// ============================================================================

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
        let commit_short = if build.commit_hash.len() >= 8 {
            build.commit_hash[..8].to_string()
        } else {
            build.commit_hash.clone()
        };

        // Calculate duration if both timestamps exist
        let duration_seconds = if let (Some(started), Some(finished)) =
            (&build.started_at, &build.finished_at)
        {
            // Parse timestamps and calculate difference
            // Format: "2024-01-01 12:00:00"
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
            id: build.id,
            repo: build.repo_name,
            commit_hash: build.commit_hash,
            commit_short,
            branch: build.branch,
            status: build.status,
            retry_count: build.retry_count,
            max_retries: build.max_retries,
            created_at: build.created_at,
            started_at: build.started_at,
            finished_at: build.finished_at,
            duration_seconds,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BuildsListResponse {
    builds: Vec<BuildInfo>,
    total: usize,
}

/// List all builds with optional filtering
pub async fn list_builds(
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
pub async fn get_build(
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

#[derive(Debug, Serialize)]
pub struct RepoInfo {
    name: String,
    path: String,
    builds_count: i64,
    last_build: Option<BuildInfo>,
}

/// List all repositories
pub async fn list_repos(
    State(state): State<AppState>,
) -> Result<Json<Vec<RepoInfo>>, (StatusCode, String)> {
    // List repos from filesystem (this is the source of truth)
    match state.repo_manager.list_repos().await {
        Ok(repos) => {
            let mut repo_infos = Vec::new();

            for (name, path) in repos {
                // Get build stats from database
                let builds_from_db = state.queue.list_repos().await.ok().unwrap_or_default();
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
                    path: path.to_string_lossy().to_string(),
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

/// Create a new bare repository
pub async fn create_repo(
    State(state): State<AppState>,
    Json(req): Json<CreateRepoRequest>,
) -> Result<(StatusCode, Json<CreateRepoResponse>), (StatusCode, String)> {
    info!("Creating repo: {}", req.name);

    // TODO: Validate repo name (alphanumeric, no special chars except - and _)
    if !req.name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid repo name. Use only alphanumeric, dash, and underscore".to_string(),
        ));
    }

    match state.repo_manager.create_bare_repo(&req.name).await {
        Ok(repo_path) => {
            info!("Repository created at: {}", repo_path);

            // Get hostname for git URL
            let hostname = hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "localhost".to_string());

            Ok((
                StatusCode::CREATED,
                Json(CreateRepoResponse {
                    name: req.name.clone(),
                    path: repo_path.clone(),
                    git_url: format!("git@{}:{}", hostname, repo_path),
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

#[derive(Debug, Serialize)]
pub struct RepoDetails {
    name: String,
    path: String,
    git_url: String,
    ssh_url: String,
    clone_url: String,
    builds_count: i64,
    recent_builds: Vec<BuildInfo>,
    setup_instructions: SetupInstructions,
    flake_metadata: Option<crate::nix_parser::FlakeMetadata>,
}

#[derive(Debug, Serialize)]
pub struct SetupInstructions {
    new_repo: Vec<String>,
    existing_repo: Vec<String>,
}

/// Get details for a specific repository
pub async fn get_repo(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<RepoDetails>, (StatusCode, String)> {
    // Check if repo exists
    let repos = state.repo_manager.list_repos().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let repo = repos.iter().find(|(repo_name, _)| repo_name == &name)
        .ok_or((StatusCode::NOT_FOUND, format!("Repository '{}' not found", name)))?;

    // Get hostname for URLs
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());

    let repo_path = repo.1.to_string_lossy().to_string();

    // Convert to absolute path if relative
    let absolute_path = if std::path::Path::new(&repo_path).is_absolute() {
        repo_path.clone()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&repo_path).to_string_lossy().to_string())
            .unwrap_or(repo_path.clone())
    };

    // Get build stats
    let all_builds = state.queue.list_all_builds().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let repo_builds: Vec<BuildInfo> = all_builds.into_iter()
        .filter(|b| b.repo_name == name)
        .take(10)
        .map(BuildInfo::from)
        .collect();

    let builds_count = repo_builds.len() as i64;

    // Try to parse flake metadata (if it's a Nix flake)
    let flake_metadata = match crate::nix_parser::get_flake_metadata(&absolute_path).await {
        Ok(metadata) => {
            info!("Parsed flake metadata for repo: {}", name);
            Some(metadata)
        }
        Err(e) => {
            info!("Could not parse flake metadata for {}: {}", name, e);
            None
        }
    };

    let details = RepoDetails {
        name: name.clone(),
        path: repo_path.clone(),
        git_url: format!("git@{}:{}", hostname, repo_path),
        ssh_url: format!("ssh://git@{}/{}.git", hostname, name),
        clone_url: format!("git@{}:{}.git", hostname, name),
        builds_count,
        recent_builds: repo_builds,
        setup_instructions: SetupInstructions {
            new_repo: vec![
                format!("echo \"# {}\" >> README.md", name),
                "git init".to_string(),
                "git add .".to_string(),
                "git commit -m \"Initial commit\"".to_string(),
                format!("git remote add origin git@{}:{}.git", hostname, name),
                "git push -u origin main".to_string(),
            ],
            existing_repo: vec![
                format!("git remote add origin git@{}:{}.git", hostname, name),
                "git push -u origin main".to_string(),
            ],
        },
        flake_metadata,
    };

    Ok(Json(details))
}

// ============================================================================
// Server-Sent Events for Real-time Updates
// ============================================================================

#[derive(Debug, Serialize)]
#[allow(dead_code)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BuildEvent {
    Created { build: BuildInfo },
    Updated { build: BuildInfo },
    Completed { build: BuildInfo },
}

/// Stream build events in real-time
pub async fn build_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<axum::response::sse::Event, Infallible>>> {
    let stream = stream::unfold(state, |state| async move {
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Poll for recent builds
        let builds = state.queue.list_all_builds().await.ok()?;

        // Just send the latest builds as a heartbeat
        if let Some(latest) = builds.first() {
            let event = BuildEvent::Updated {
                build: BuildInfo::from(latest.clone()),
            };

            let data = serde_json::to_string(&event).ok()?;
            let sse_event = axum::response::sse::Event::default().data(data);

            Some((Ok(sse_event), state))
        } else {
            // Keep connection alive with comment
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

// ============================================================================
// Build Log Retrieval
// ============================================================================

/// Get build logs (if stored)
pub async fn get_build_logs(
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
