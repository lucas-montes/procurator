//! Check and log endpoints.

use std::fmt::Write as _;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use super::error::HttpError;
use super::shared::Shared;
use crate::application::ports::{Builder, Clock, ConfigSource, Notifier, RunStore};
use crate::domain::check::{Check, Derivation};
use crate::domain::log::{LogLine, LogOrigin};
use crate::domain::run::RunId;

/// A check as JSON, borrowing from the domain value.
#[derive(Serialize)]
struct CheckBody<'a> {
    attr: &'a str,
    system: &'a str,
    status: &'static str,
    drv_path: Option<&'a str>,
    machine: Option<&'a str>,
    tries: u32,
}

impl<'a> From<&'a Check> for CheckBody<'a> {
    fn from(check: &'a Check) -> Self {
        Self {
            attr: check.attr(),
            system: check.system(),
            status: check.status().as_str(),
            drv_path: check.derivation().map(Derivation::path),
            machine: check.machine(),
            tries: check.tries(),
        }
    }
}

#[derive(Serialize)]
struct ChecksBody<'a> {
    checks: Vec<CheckBody<'a>>,
}

/// Lines as plain text. With `labels`, each line from a derivation or check
/// is prefixed with its name, as in `nix build -L`.
fn text(lines: &[LogLine], labels: bool) -> Response {
    let mut body = String::new();
    for line in lines {
        let origin = line.origin();
        if labels && origin != LogOrigin::Run {
            let _ = write!(body, "{}> ", origin.label());
        }
        body.push_str(line.text());
        body.push('\n');
    }
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

/// `GET /api/runs/{id}/checks`: a run's checks, in evaluation order.
pub async fn list<S, C, B, R, N>(
    State(shared): State<Shared<S, C, B, R, N>>,
    Path(id): Path<i64>,
) -> Result<Response, HttpError>
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
    N: Notifier,
{
    let checks = shared.service().checks(RunId::new(id)).await?;
    let body = ChecksBody {
        checks: checks.iter().map(CheckBody::from).collect(),
    };
    Ok(Json(body).into_response())
}

/// `GET /api/runs/{id}/log`: everything the run logged, labelled.
pub async fn run_log<S, C, B, R, N>(
    State(shared): State<Shared<S, C, B, R, N>>,
    Path(id): Path<i64>,
) -> Result<Response, HttpError>
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
    N: Notifier,
{
    let lines = shared.service().run_log(RunId::new(id)).await?;
    Ok(text(&lines, true))
}

/// `GET /api/runs/{id}/checks/{attr}/log`: one check's own log.
pub async fn log<S, C, B, R, N>(
    State(shared): State<Shared<S, C, B, R, N>>,
    Path((id, attr)): Path<(i64, String)>,
) -> Result<Response, HttpError>
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
    N: Notifier,
{
    let lines = shared.service().check_log(RunId::new(id), &attr).await?;
    Ok(text(&lines, false))
}
