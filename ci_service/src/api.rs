use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{
    builds::{BuildInfo, BuildStatus},
    job_queue::JobQueue,
};

#[derive(Debug, Serialize)]
pub struct BuildsListResponse {
    builds: Vec<BuildInfo>,
    total: usize,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
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

#[derive(Clone)]
pub struct AppState {
    queue: JobQueue,
}

impl AppState {
    pub fn new(queue: JobQueue) -> Self {
        Self { queue }
    }
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

    //TODO: check if this works as expected. Maybe we want to use username/reponame

    // Enqueue build with bare repo path
    match state
        .queue
        .enqueue(&req.bare_repo_path, &req.new_rev, branch)
        .await
    {
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

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/builds", post(create_build).get(list_builds))
        .route("/builds/{id}", get(get_build))
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
