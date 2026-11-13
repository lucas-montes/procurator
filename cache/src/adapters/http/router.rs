//! The HTTP router.
//! Why: it only extracts credentials and maps errors — every access decision
//! is made by the application layer, so other interfaces behave identically.
//!
//! Request flow: [`log_request`] wraps everything; [`authenticate`] turns the
//! `Authorization` header into a principal once per request; handlers
//! receive it through the [`Caller`](super::auth::Caller) extractor.

use std::sync::Arc;

use axum::Router;
use axum::middleware;
use axum::routing::get;

use super::auth::authenticate;
use super::request_log::log_request;
use super::{admin, nix, ops};
use crate::application::Service;
use crate::application::ports::{BlobStore, Clock, Entropy, Index};

/// Build the router for a service. Mount it at the root of a listener, or
/// nest it inside another axum application.
pub fn router<B, I, E>(service: Arc<Service<B, I, E>>) -> Router
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    let authenticated = Router::new()
        .route("/metrics", get(ops::metrics::<B, I, E>))
        .merge(admin::routes())
        .merge(nix::routes())
        // Why: `route_layer` runs only for matched routes, so unknown paths
        // stay 404 instead of turning into a credentials check.
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&service),
            authenticate::<B, I, E>,
        ));
    Router::new()
        // Why: probes stay outside auth — a stale token on a load balancer
        // must not mark the service down.
        .route("/healthz", get(ops::healthz))
        .route("/readyz", get(ops::readyz::<B, I, E>))
        .merge(authenticated)
        .layer(middleware::from_fn(log_request))
        .with_state(service)
}
