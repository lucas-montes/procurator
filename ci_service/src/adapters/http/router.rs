//! The HTTP router.
//! Why: handlers only parse and map errors — every rule lives in the
//! application layer, so direct callers in the monolith behave identically.

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use super::{checks, runs};
use crate::application::Service;
use crate::application::ports::{Builder, Clock, RunStore};

/// Build the router for a service. Mount it at the root of a listener, or
/// nest it inside the monolith's axum application.
pub fn router<S, C, B>(service: Arc<Service<S, C, B>>) -> Router
where
    S: RunStore,
    C: Clock,
    B: Builder,
{
    Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/api/runs", post(runs::create::<S, C, B>))
        .route("/api/runs/{id}", get(runs::show::<S, C, B>))
        .route("/api/runs/{id}/log", get(checks::run_log::<S, C, B>))
        .route("/api/runs/{id}/checks", get(checks::list::<S, C, B>))
        .route(
            "/api/runs/{id}/checks/{attr}/log",
            get(checks::log::<S, C, B>),
        )
        .route("/api/repos/{owner}/{name}/runs", get(runs::list::<S, C, B>))
        .with_state(service)
}
