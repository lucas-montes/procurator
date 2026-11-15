//! The HTTP router.
//! Why: handlers only parse and map errors — every rule lives in the
//! application layer, so direct callers in the monolith behave identically.

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use super::runs;
use crate::application::Service;
use crate::application::ports::{Clock, RunStore};

/// Build the router for a service. Mount it at the root of a listener, or
/// nest it inside the monolith's axum application.
pub fn router<S, C>(service: Arc<Service<S, C>>) -> Router
where
    S: RunStore,
    C: Clock,
{
    Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/api/runs", post(runs::create::<S, C>))
        .route("/api/runs/{id}", get(runs::show::<S, C>))
        .route("/api/repos/{owner}/{name}/runs", get(runs::list::<S, C>))
        .with_state(service)
}
