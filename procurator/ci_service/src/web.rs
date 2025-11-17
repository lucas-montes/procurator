use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response, Sse},
    Json,
};
use futures::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::StreamExt as _;
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
            repo: build.repo,
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

// ============================================================================
// Server-Sent Events for Real-time Updates
// ============================================================================

#[derive(Debug, Serialize)]
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
) -> Result<Response, (StatusCode, String)> {
    match state.queue.get_build_logs(id).await {
        Ok(Some(logs)) => Ok(logs.into_response()),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            "No logs available for this build".to_string(),
        )),
        Err(e) => {
            tracing::error!("Failed to get logs for build {}: {}", id, e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get logs: {}", e),
            ))
        }
    }
}
