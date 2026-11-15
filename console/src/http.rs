//! HTTP surface: one HTML shell (the UI routes by URL in `app.js`), its two
//! assets, the model snapshot, and one live SSE stream carrying `state`,
//! `message` and `log` events. One stream per page matters: browsers allow
//! only six HTTP/1.1 connections per host. Read-only: there is no route that
//! changes anything.

use std::{convert::Infallible, net::SocketAddr};

use axum::{
    Router,
    extract::State,
    http::header,
    response::{
        Html, IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use futures::{Stream, stream};
use tokio::sync::{broadcast, watch};

const INDEX: &str = include_str!("../assets/index.html");
const APP_JS: &str = include_str!("../assets/app.js");
const APP_CSS: &str = include_str!("../assets/app.css");

#[derive(Clone)]
struct AppState {
    snapshot: watch::Receiver<String>,
    messages: broadcast::Sender<String>,
    logs: broadcast::Sender<String>,
}

/// Serves the console until the listener fails.
///
/// # Errors
///
/// Returns the bind or serve error.
pub async fn serve(
    addr: SocketAddr,
    snapshot: watch::Receiver<String>,
    messages: broadcast::Sender<String>,
    logs: broadcast::Sender<String>,
) -> std::io::Result<()> {
    let state = AppState {
        snapshot,
        messages,
        logs,
    };
    let router = Router::new()
        .route("/api/model", get(model))
        .route("/api/stream", get(live_stream))
        .route("/assets/app.js", get(app_js))
        .route("/assets/app.css", get(app_css))
        .fallback(get(index))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "console listening");
    axum::serve(listener, router).await
}

async fn index() -> Html<&'static str> {
    Html(INDEX)
}

async fn app_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        APP_JS,
    )
}

async fn app_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], APP_CSS)
}

async fn model(State(state): State<AppState>) -> impl IntoResponse {
    let body = state.snapshot.borrow().clone();
    ([(header::CONTENT_TYPE, "application/json")], body)
}

/// The snapshot now and on every change (`state`), plus live `message` and
/// `log` events. A browser that falls behind gets a `lagged` event with the
/// number of skipped events.
async fn live_stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut snapshot = state.snapshot;
    snapshot.mark_changed();
    let states = stream::unfold(snapshot, |mut receiver| async move {
        receiver.changed().await.ok()?;
        let data = receiver.borrow_and_update().clone();
        Some((Ok(Event::default().event("state").data(data)), receiver))
    });
    let messages = broadcast_events(state.messages.subscribe(), "message");
    let logs = broadcast_events(state.logs.subscribe(), "log");
    let events = stream::select(states, stream::select(messages, logs));
    Sse::new(events).keep_alive(KeepAlive::default())
}

fn broadcast_events(
    receiver: broadcast::Receiver<String>,
    name: &'static str,
) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::unfold(receiver, move |mut receiver| async move {
        let event = match receiver.recv().await {
            Ok(data) => Event::default().event(name).data(data),
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                Event::default().event("lagged").data(skipped.to_string())
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        };
        Some((Ok(event), receiver))
    })
}
