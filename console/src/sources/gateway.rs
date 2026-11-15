//! `systemd-journal-gatewayd`: `GET /entries?follow&_SYSTEMD_UNIT=<unit>`
//! with `Accept: application/json` streams one entry per line. The first
//! request reads back the last `entries` entries (`Range: entries=:-N:`);
//! reconnects resume right after the last cursor seen
//! (`Range: entries=<cursor>:1:`), so nothing is duplicated or lost while the
//! journal still holds it.

use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::Request;
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use tokio::{sync::mpsc::Sender, time::sleep};

use super::{
    event::{SourceEvent, line, status},
    journal_entry,
};

/// Capped backoff between reconnects.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Follows the unit's journal on `url` forever.
pub async fn follow_gateway(
    source: usize,
    url: String,
    unit: String,
    entries: u64,
    tx: Sender<SourceEvent>,
) {
    let client: Client<_, Empty<Bytes>> = Client::builder(TokioExecutor::new()).build_http();
    let endpoint = format!(
        "{}/entries?follow&_SYSTEMD_UNIT={unit}",
        url.trim_end_matches('/')
    );
    let mut cursor: Option<String> = None;
    let mut failures: u32 = 0;
    loop {
        match stream_once(&client, &endpoint, entries, &mut cursor, source, &tx).await {
            Ok(Stop::ModelGone) => return,
            Ok(Stop::Ended) => {
                failures = 0;
                status(&tx, source, Some("gateway closed the stream".to_owned())).await;
            }
            Err(err) => {
                failures = failures.saturating_add(1);
                status(&tx, source, Some(err)).await;
            }
        }
        let delay = Duration::from_secs(1u64 << failures.min(5)).min(MAX_BACKOFF);
        sleep(delay).await;
    }
}

enum Stop {
    Ended,
    ModelGone,
}

async fn stream_once(
    client: &Client<hyper_util::client::legacy::connect::HttpConnector, Empty<Bytes>>,
    endpoint: &str,
    entries: u64,
    cursor: &mut Option<String>,
    source: usize,
    tx: &Sender<SourceEvent>,
) -> Result<Stop, String> {
    let range = match cursor.as_deref() {
        Some(cursor) => format!("entries={cursor}:1:"),
        None => format!("entries=:-{entries}:"),
    };
    let request = Request::get(endpoint)
        .header("Accept", "application/json")
        .header("Range", range)
        .body(Empty::new())
        .map_err(|err| format!("bad gateway url: {err}"))?;
    let response = client
        .request(request)
        .await
        .map_err(|err| format!("cannot reach gateway: {err}"))?;
    if !response.status().is_success() {
        return Err(format!("gateway answered {}", response.status()));
    }
    status(tx, source, None).await;

    let mut body = response.into_body();
    let mut pending: Vec<u8> = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|err| format!("gateway stream broke: {err}"))?;
        let Some(data) = frame.data_ref() else {
            continue;
        };
        pending.extend_from_slice(data);
        let mut start = 0;
        while let Some(offset) = pending[start..].iter().position(|byte| *byte == b'\n') {
            let end = start + offset;
            let text = String::from_utf8_lossy(&pending[start..end]);
            start = end + 1;
            let Some(entry) = journal_entry::parse(&text) else {
                continue;
            };
            let (message, entry_cursor) = entry.into_parts();
            if entry_cursor.is_some() {
                *cursor = entry_cursor;
            }
            if let Some(message) = message
                && !line(tx, source, message).await
            {
                return Ok(Stop::ModelGone);
            }
        }
        pending.drain(..start);
    }
    Ok(Stop::Ended)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::{Router, http::HeaderMap, routing::get};
    use tokio::{sync::mpsc, time::timeout};

    use super::{SourceEvent, follow_gateway};

    /// A gateway that answers each request with the next cursor's entry and
    /// closes the stream, recording the `Range` headers it was sent.
    async fn stub_gateway() -> (String, mpsc::UnboundedReceiver<String>) {
        let (ranges_tx, ranges_rx) = mpsc::unbounded_channel();
        let router = Router::new().route(
            "/entries",
            get(move |headers: HeaderMap| {
                let ranges_tx = ranges_tx.clone();
                async move {
                    let range = headers
                        .get("range")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned();
                    let next = if range.contains("c1") { 2 } else { 1 };
                    let _ = ranges_tx.send(range);
                    format!("{{\"__CURSOR\":\"c{next}\",\"MESSAGE\":\"line {next}\"}}\n")
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move { axum::serve(listener, router).await });
        (format!("http://{addr}"), ranges_rx)
    }

    async fn next_line(rx: &mut mpsc::Receiver<SourceEvent>) -> String {
        loop {
            match timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("event in time")
            {
                Some(SourceEvent::Line { text, .. }) => return text,
                Some(SourceEvent::Status { .. }) => {}
                None => panic!("source stopped"),
            }
        }
    }

    #[tokio::test]
    async fn reads_back_then_resumes_after_the_last_cursor() {
        let (url, mut ranges) = stub_gateway().await;
        let (tx, mut rx) = mpsc::channel(16);
        let task = tokio::spawn(follow_gateway(0, url, "svc.service".to_owned(), 7, tx));

        assert_eq!(next_line(&mut rx).await, "line 1");
        assert_eq!(ranges.recv().await.as_deref(), Some("entries=:-7:"));
        // The stream ended; the reconnect resumes right after cursor c1.
        assert_eq!(next_line(&mut rx).await, "line 2");
        assert_eq!(ranges.recv().await.as_deref(), Some("entries=c1:1:"));
        task.abort();
    }
}
