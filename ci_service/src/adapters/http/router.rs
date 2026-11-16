//! The HTTP router.
//! Why: handlers only parse and map errors — every rule lives in the
//! application layer, so direct callers in the monolith behave identically.

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use super::shared::Shared;
use super::{checks, runs};
use crate::application::Service;
use crate::application::ports::{Access, Builder, Clock, ConfigSource, Notifier, RunStore};

/// Build the router for a service. Mount it at the root of a listener, or
/// merge it into the monolith's axum application.
pub fn router<S, C, B, R, N, A>(service: Arc<Service<S, C, B, R, N, A>>) -> Router
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
    N: Notifier,
    A: Access,
{
    Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/api/runs", post(runs::create::<S, C, B, R, N, A>))
        .route("/api/runs/{id}", get(runs::show::<S, C, B, R, N, A>))
        .route(
            "/api/runs/{id}/log",
            get(checks::run_log::<S, C, B, R, N, A>),
        )
        .route(
            "/api/runs/{id}/checks",
            get(checks::list::<S, C, B, R, N, A>),
        )
        .route(
            "/api/runs/{id}/checks/{attr}/log",
            get(checks::log::<S, C, B, R, N, A>),
        )
        .route(
            "/api/repos/{owner}/{name}/runs",
            get(runs::list::<S, C, B, R, N, A>),
        )
        .with_state(Shared::new(service))
}
