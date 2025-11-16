use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct BuildRequest {
    pub repo: String,
    pub old_rev: String,
    pub new_rev: String,
    #[serde(rename = "ref")]
    pub ref_name: String,
}

#[derive(Debug, Serialize)]
pub struct BuildResponse {
    pub id: i64,
    pub status: String,
}

pub async fn create_build(
    State(state): State<AppState>,
    Json(req): Json<BuildRequest>,
) -> Result<(StatusCode, Json<BuildResponse>), (StatusCode, String)> {
    let commit_short = if req.new_rev.len() >= 8 {
        &req.new_rev[..8]
    } else {
        &req.new_rev
    };

    info!(
        "Build request: repo={} ref={} commit={}",
        req.repo, req.ref_name, commit_short
    );

    // Extract branch name
    let branch = req
        .ref_name
        .strip_prefix("refs/heads/")
        .unwrap_or(&req.ref_name);

    // Enqueue build
    match state.queue.enqueue(&req.repo, &req.new_rev, branch).await {
        Ok(id) => {
            info!("Build #{} queued for {}/{}", id, req.repo, branch);
            Ok((
                StatusCode::ACCEPTED,
                Json(BuildResponse {
                    id,
                    status: "queued".to_string(),
                }),
            ))
        }
        Err(e) => {
            tracing::error!("Failed to enqueue build: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to enqueue build: {}", e),
            ))
        }
    }
}
