//! Operational endpoints: probes and metrics.

use std::fmt::Write as _;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use super::auth::Caller;
use super::error::HttpError;
use crate::application::Service;
use crate::application::ports::{BlobStore, Clock, Entropy, Index};

/// Liveness: the process answers. Constant time, no dependencies.
pub async fn healthz() -> &'static str {
    "ok\n"
}

/// Readiness: the index answers.
pub async fn readyz<B, I, E>(State(service): State<Arc<Service<B, I, E>>>) -> Response
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    match service.ready().await {
        Ok(()) => "ok\n".into_response(),
        Err(error) => {
            tracing::warn!(%error, "not ready");
            (StatusCode::SERVICE_UNAVAILABLE, "not ready\n").into_response()
        }
    }
}

/// Per-cache gauges in Prometheus text format. Admins only: sizes of
/// private caches are not public information.
pub async fn metrics<B, I, E>(
    State(service): State<Arc<Service<B, I, E>>>,
    caller: Caller,
) -> Result<String, HttpError>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    let mut out = String::from(
        "# TYPE procurator_cache_bytes gauge\n\
         # TYPE procurator_cache_quota_bytes gauge\n\
         # TYPE procurator_cache_nars gauge\n\
         # TYPE procurator_cache_paths gauge\n",
    );
    for spec in service.caches() {
        let name = spec.name().as_str();
        let usage = service.usage(caller.principal(), name).await?;
        let _ = writeln!(
            out,
            "procurator_cache_bytes{{cache=\"{name}\"}} {}",
            usage.bytes()
        );
        let _ = writeln!(
            out,
            "procurator_cache_quota_bytes{{cache=\"{name}\"}} {}",
            spec.quota_bytes()
        );
        let _ = writeln!(
            out,
            "procurator_cache_nars{{cache=\"{name}\"}} {}",
            usage.nars()
        );
        let _ = writeln!(
            out,
            "procurator_cache_paths{{cache=\"{name}\"}} {}",
            usage.narinfos()
        );
    }
    Ok(out)
}
