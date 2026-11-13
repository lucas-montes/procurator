//! Access logging middleware.

use std::time::Instant;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

/// One log line per request.
pub async fn log_request(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = String::from(request.uri().path());
    let started = Instant::now();
    let response = next.run(request).await;
    tracing::info!(
        %method,
        path,
        status = response.status().as_u16(),
        elapsed_ms = started.elapsed().as_millis(),
        "request"
    );
    response
}
