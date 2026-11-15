//! Run endpoints: the hook enqueues, UIs read.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use super::error::HttpError;
use crate::application::ports::{Builder, Clock, ConfigSource, RunStore};
use crate::application::{Service, ServiceError};
use crate::domain::run::{CommitHash, GitRef, NewRun, Pusher, RepoName, Run, RunId};

/// Default page size for run listings.
const DEFAULT_PAGE: u32 = 20;

/// What the post-receive hook sends.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    repo: String,
    commit: String,
    #[serde(rename = "ref")]
    git_ref: String,
    #[serde(default)]
    pusher: Option<String>,
}

impl CreateBody {
    /// Validate into a domain request, moving the strings in.
    fn into_request(self) -> Result<NewRun, ServiceError> {
        Ok(NewRun::new(
            RepoName::parse(self.repo)?,
            CommitHash::parse(self.commit)?,
            GitRef::parse(self.git_ref)?,
            self.pusher.map(Pusher::parse).transpose()?,
        ))
    }
}

#[derive(Serialize)]
struct CreatedBody {
    id: i64,
    status: &'static str,
}

/// A run as JSON, borrowing from the domain value.
#[derive(Serialize)]
struct RunBody<'a> {
    id: i64,
    repo: &'a str,
    commit: &'a str,
    #[serde(rename = "ref")]
    git_ref: &'a str,
    pusher: Option<&'a str>,
    status: &'static str,
    created_at: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
}

impl<'a> From<&'a Run> for RunBody<'a> {
    fn from(run: &'a Run) -> Self {
        let request = run.request();
        let times = run.times();
        Self {
            id: run.id().get(),
            repo: request.repo().as_str(),
            commit: request.commit().as_str(),
            git_ref: request.git_ref().as_str(),
            pusher: request.pusher().map(Pusher::as_str),
            status: run.status().as_str(),
            created_at: times.created_at(),
            started_at: times.started_at(),
            finished_at: times.finished_at(),
        }
    }
}

#[derive(Serialize)]
struct RunsBody<'a> {
    runs: Vec<RunBody<'a>>,
}

/// Paging for run listings.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListParams {
    #[serde(default)]
    before: Option<i64>,
    #[serde(default)]
    limit: Option<u32>,
}

/// `POST /api/runs`: queue a run. Answers 202 with the run id.
pub async fn create<S, C, B, R>(
    State(service): State<Arc<Service<S, C, B, R>>>,
    Json(body): Json<CreateBody>,
) -> Result<Response, HttpError>
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
{
    let request = body.into_request()?;
    let id = service.enqueue(request).await?;
    tracing::info!(run = %id, "run queued");
    let created = CreatedBody {
        id: id.get(),
        status: "queued",
    };
    Ok((StatusCode::ACCEPTED, Json(created)).into_response())
}

/// `GET /api/runs/{id}`: one run.
pub async fn show<S, C, B, R>(
    State(service): State<Arc<Service<S, C, B, R>>>,
    Path(id): Path<i64>,
) -> Result<Response, HttpError>
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
{
    let run = service.run(RunId::new(id)).await?;
    Ok(Json(RunBody::from(&run)).into_response())
}

/// `GET /api/repos/{owner}/{name}/runs`: a repo's runs, newest first.
pub async fn list<S, C, B, R>(
    State(service): State<Arc<Service<S, C, B, R>>>,
    Path((owner, name)): Path<(String, String)>,
    Query(params): Query<ListParams>,
) -> Result<Response, HttpError>
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
{
    let repo = RepoName::parse(format!("{owner}/{name}")).map_err(ServiceError::from)?;
    let before = params.before.map(RunId::new);
    let limit = params.limit.unwrap_or(DEFAULT_PAGE);
    let runs = service.runs(Some(&repo), before, limit).await?;
    let body = RunsBody {
        runs: runs.iter().map(RunBody::from).collect(),
    };
    Ok(Json(body).into_response())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt as _;
    use serde_json::Value;
    use tower::ServiceExt as _;

    use crate::adapters::http::router;
    use crate::adapters::memory::MemoryRunStore;
    use crate::adapters::scripted::{FixedConfig, ScriptedBuilder, derivation};
    use crate::adapters::system::SystemClock;
    use crate::application::Service;
    use crate::application::ports::BuildEvent;
    use crate::domain::repo_config::Limits;

    const DRV: &str = "/nix/store/u-unit.drv";

    fn service() -> Arc<Service<MemoryRunStore, SystemClock, ScriptedBuilder, FixedConfig>> {
        let evaluation = vec![derivation(
            "checks.x86_64-linux.unit",
            "x86_64-linux",
            DRV,
            false,
        )];
        let events = vec![
            BuildEvent::Started {
                build: 1,
                drv: String::from(DRV),
                machine: String::new(),
            },
            BuildEvent::Line {
                build: 1,
                text: String::from("unit ok"),
            },
        ];
        let builder = ScriptedBuilder::new(&["x86_64-linux"], Ok(evaluation))
            .with_events(events)
            .passing(&[DRV]);
        Arc::new(Service::new(
            MemoryRunStore::new(),
            SystemClock,
            builder,
            FixedConfig::default(),
            Limits::new(1, 60, 60),
        ))
    }

    fn app() -> axum::Router {
        router(service())
    }

    async fn text(app: &axum::Router, uri: &str) -> (StatusCode, String) {
        let response = app.clone().oneshot(get(uri)).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn a_worked_run_shows_checks_and_logs() {
        let service = service();
        let app = router(Arc::clone(&service));
        let body =
            format!(r#"{{"repo":"lucas/dummy","commit":"{COMMIT}","ref":"refs/heads/main"}}"#);
        let (_, created) = send(&app, post(&body)).await;
        let id = created["id"].as_i64().unwrap();
        assert!(service.work_next().await.unwrap());

        let (status, run) = send(&app, get(&format!("/api/runs/{id}"))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(run["status"], "passed");

        let (status, checks) = send(&app, get(&format!("/api/runs/{id}/checks"))).await;
        assert_eq!(status, StatusCode::OK);
        let check = &checks["checks"][0];
        assert_eq!(check["attr"], "checks.x86_64-linux.unit");
        assert_eq!(check["status"], "passed");
        assert_eq!(check["machine"], "local");
        assert_eq!(check["tries"], 1);
        assert_eq!(check["drv_path"], DRV);

        let uri = format!("/api/runs/{id}/checks/checks.x86_64-linux.unit/log");
        assert_eq!(
            text(&app, &uri).await,
            (StatusCode::OK, String::from("unit ok\n"))
        );
        let run_log = text(&app, &format!("/api/runs/{id}/log")).await;
        assert_eq!(run_log, (StatusCode::OK, String::from("unit> unit ok\n")));

        let missing = text(&app, &format!("/api/runs/{id}/checks/checks.nope/log")).await;
        assert_eq!(missing.0, StatusCode::NOT_FOUND);
        let (status, _) = send(&app, get("/api/runs/999/checks")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, Value) {
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    fn post(body: &str) -> Request<Body> {
        Request::post("/api/runs")
            .header("content-type", "application/json")
            .body(Body::from(String::from(body)))
            .unwrap()
    }

    fn get(uri: &str) -> Request<Body> {
        Request::get(uri).body(Body::empty()).unwrap()
    }

    const COMMIT: &str = "1e43a4529500115f72383235e7112e4e3ba91005";

    #[tokio::test]
    async fn hook_payload_queues_a_readable_run() {
        let app = app();
        let body = format!(
            r#"{{"repo":"lucas/dummy","commit":"{COMMIT}","ref":"refs/heads/master","pusher":"git"}}"#
        );
        let (status, created) = send(&app, post(&body)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(created["status"], "queued");
        let id = created["id"].as_i64().unwrap();

        let (status, run) = send(&app, get(&format!("/api/runs/{id}"))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(run["repo"], "lucas/dummy");
        assert_eq!(run["commit"], COMMIT);
        assert_eq!(run["ref"], "refs/heads/master");
        assert_eq!(run["pusher"], "git");
        assert_eq!(run["status"], "queued");

        let (status, list) = send(&app, get("/api/repos/lucas/dummy/runs")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["runs"].as_array().unwrap().len(), 1);
        let (_, other) = send(&app, get("/api/repos/lucas/other/runs")).await;
        assert!(other["runs"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_filesystem_path_is_not_a_repo() {
        let body = format!(
            r#"{{"repo":"/var/lib/git-server/lucas/dummy.git","commit":"{COMMIT}","ref":"refs/heads/master"}}"#
        );
        let (status, _) = send(&app(), post(&body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_run_is_404_and_health_is_ok() {
        let app = app();
        let (status, _) = send(&app, get("/api/runs/42")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let response = app.oneshot(get("/health")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
