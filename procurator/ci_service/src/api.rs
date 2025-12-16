
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Sse,
    routing::{get, post},
    Json, Router,
};
use futures::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc};
use std::path::PathBuf;
use std::time::Duration;
use tracing::info;

use crate::{builds::BuildStatus, config::Config, job_queue::JobQueue};


#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum BuildEvent {
    Created { build: BuildInfo },
    Updated { build: BuildInfo },
    Completed { build: BuildInfo },
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


#[derive(Clone)]
pub struct AppState {
    queue: Arc<JobQueue>,
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

/// Stream build events in real-time
// async fn build_events(
//     State(state): State<AppState>,
// ) -> Sse<impl Stream<Item = Result<axum::response::sse::Event, Infallible>>> {
//     let stream = stream::unfold(state, |state| async move {
//         tokio::time::sleep(Duration::from_secs(2)).await;

//         let builds = state.queue.list_all_builds().await.ok()?;

//         if let Some(latest) = builds.first() {
//             let event = BuildEvent::Updated {
//                 build: BuildInfo::from(latest.clone()),
//             };

//             let data = serde_json::to_string(&event).ok()?;
//             let sse_event = axum::response::sse::Event::default().data(data);

//             Some((Ok(sse_event), state))
//         } else {
//             let sse_event = axum::response::sse::Event::default().comment("ping");
//             Some((Ok(sse_event), state))
//         }
//     });

//     Sse::new(stream).keep_alive(
//         axum::response::sse::KeepAlive::new()
//             .interval(Duration::from_secs(30))
//             .text("keep-alive"),
//     )
// }

/// Build the API routes
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
