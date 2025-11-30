//! CI API Handlers
//!
//! This module provides HTTP API endpoints for the CI service:
//! - `POST /api/builds` - Create a new build (called by Git post-receive hooks)
//!
//! The build request contains:
//! - Repository information (name, path)
//! - Git ref and commit information
//! - Optional author/email/message metadata
//! - Optional GPG signing information
//!
//! Builds are enqueued asynchronously and processed by the worker in the background.

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::queue::BuildStatus;
use crate::AppState;

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
        repo = req.repo.as_str(),
        ref_name = req.ref_name.as_str(),
        commit = commit_short,
        author = req.commit_author.as_deref().unwrap_or("unknown"),
        gpg_status = req.gpg_status.as_deref().unwrap_or("N"),
        "Build request received"
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
