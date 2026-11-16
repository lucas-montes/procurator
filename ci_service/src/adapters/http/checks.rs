//! Check and log endpoints. Logs are plain text, or Server-Sent Events
//! with `?follow=true`: what is stored, then each line as it comes, until
//! the run ends (one last `end` event).

use std::convert::Infallible;
use std::fmt::Write as _;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::stream;
use serde::{Deserialize, Serialize};

use super::credential::credential;
use super::error::HttpError;
use super::shared::Shared;
use crate::application::LogFollow;
use crate::application::ports::{Access, Builder, Clock, ConfigSource, Notifier, RunStore};
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

/// `?follow=true` streams instead of answering once.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogParams {
    #[serde(default)]
    follow: bool,
}

/// One streamed line: `origin` is the derivation or check name, empty for
/// the run itself.
#[derive(Serialize)]
struct LineBody<'a> {
    seq: i64,
    origin: &'a str,
    text: &'a str,
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

/// A line as an SSE event, its `id` the line's sequence number.
fn event(line: &LogLine) -> Event {
    let body = LineBody {
        seq: line.seq(),
        origin: line.origin().label(),
        text: line.text(),
    };
    let event = Event::default().id(line.seq().to_string());
    match serde_json::to_string(&body) {
        Ok(json) => event.data(json),
        // Why: three plain fields always serialize; never drop the line.
        Err(_) => event.data(line.text()),
    }
}

/// Stream a followed log as Server-Sent Events.
fn follow<S, C, B, R, N, A>(shared: Shared<S, C, B, R, N, A>, follow: LogFollow) -> Response
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
    N: Notifier,
    A: Access,
{
    let events = stream::unfold(Some((shared, follow)), |state| async move {
        let (shared, mut follow) = state?;
        match shared.service().follow_next(&mut follow).await {
            Ok(Some(line)) => {
                let event = event(&line);
                Some((Ok::<_, Infallible>(event), Some((shared, follow))))
            }
            // Why: without it a browser's EventSource reconnects and
            // replays the whole log.
            Ok(None) => Some((Ok(Event::default().event("end").data("run over")), None)),
            Err(error) => {
                tracing::error!(%error, "log stream failed");
                None
            }
        }
    });
    Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// `GET /api/runs/{id}/checks`: a run's checks, in evaluation order.
pub async fn list<S, C, B, R, N, A>(
    State(shared): State<Shared<S, C, B, R, N, A>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, HttpError>
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
    N: Notifier,
    A: Access,
{
    let credential = credential(&headers)?;
    let checks = shared.service().checks(credential, RunId::new(id)).await?;
    let body = ChecksBody {
        checks: checks.iter().map(CheckBody::from).collect(),
    };
    Ok(Json(body).into_response())
}

/// `GET /api/runs/{id}/log`: everything the run logged, labelled.
pub async fn run_log<S, C, B, R, N, A>(
    State(shared): State<Shared<S, C, B, R, N, A>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(params): Query<LogParams>,
) -> Result<Response, HttpError>
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
    N: Notifier,
    A: Access,
{
    let credential = credential(&headers)?;
    let run = RunId::new(id);
    if params.follow {
        let followed = shared.service().follow_run_log(credential, run).await?;
        return Ok(follow(shared, followed));
    }
    let lines = shared.service().run_log(credential, run).await?;
    Ok(text(&lines, true))
}

/// `GET /api/runs/{id}/checks/{attr}/log`: one check's log, with the
/// builds of the dependencies it needed.
pub async fn log<S, C, B, R, N, A>(
    State(shared): State<Shared<S, C, B, R, N, A>>,
    headers: HeaderMap,
    Path((id, attr)): Path<(i64, String)>,
    Query(params): Query<LogParams>,
) -> Result<Response, HttpError>
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
    N: Notifier,
    A: Access,
{
    let credential = credential(&headers)?;
    let run = RunId::new(id);
    if params.follow {
        let followed = shared
            .service()
            .follow_check_log(credential, run, &attr)
            .await?;
        return Ok(follow(shared, followed));
    }
    let lines = shared.service().check_log(credential, run, &attr).await?;
    // Why: with its dependencies in it, each line says which build it is.
    Ok(text(&lines, true))
}
