//! Web UI Handlers
//!
//! Serves the HTML interface for the CI service:
//! - `GET /` - Single Page Application (SPA)

use axum::{response::Html, routing::get, Router};

use crate::AppState;

async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

/// Build the web UI routes
pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(index))
}
