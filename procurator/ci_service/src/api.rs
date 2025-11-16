use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::queue::BuildStatus;
use crate::AppState;

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
}

#[derive(Debug, Serialize)]
pub struct BuildResponse {
    id: i64,
    status: BuildStatus,
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
        "Build request: repo={} ref={} commit={} author={} gpg={}",
        req.repo,
        req.ref_name,
        commit_short,
        req.commit_author.as_deref().unwrap_or("unknown"),
        req.gpg_status.as_deref().unwrap_or("N")
    );

    // Extract branch name
    let branch = req
        .ref_name
        .strip_prefix("refs/heads/")
        .unwrap_or(&req.ref_name);

    // Enqueue build with bare repo path
    match state
        .queue
        .enqueue(&req.bare_repo_path, &req.new_rev, branch)
        .await
    {
        Ok(id) => {
            info!("Build #{} queued for {}/{}", id, req.repo, branch);
            Ok((
                StatusCode::ACCEPTED,
                Json(BuildResponse {
                    id,
                    status: BuildStatus::Queued,
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
