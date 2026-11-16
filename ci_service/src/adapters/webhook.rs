//! [`Notifier`] for webhooks: POST the event's JSON to the URL its secret
//! names, retrying with backoff a few times, then giving up.

use std::future::Future;
use std::time::Duration;

use crate::application::ports::{DeliveryError, Notifier, SecretStore};
use crate::domain::event::{EventPayload, Target};

/// Longest a POST may take.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Waits before the 2nd, 3rd, … attempt; after the last, give up.
const BACKOFF: [Duration; 4] = [
    Duration::from_secs(30),
    Duration::from_secs(120),
    Duration::from_secs(600),
    Duration::from_secs(1800),
];

/// An error and every cause under it, e.g. "error sending request:
/// connection refused". Why: the top level alone says nothing useful.
fn with_causes(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(inner) = cause {
        text.push_str(": ");
        text.push_str(&inner.to_string());
        cause = inner.source();
    }
    text
}

/// Delivers events to webhooks.
#[derive(Debug)]
pub struct WebhookNotifier<K> {
    client: reqwest::Client,
    secrets: K,
}

impl<K: SecretStore> WebhookNotifier<K> {
    /// A notifier resolving webhook URLs through `secrets`.
    ///
    /// # Errors
    ///
    /// When the HTTP client cannot be built (no TLS backend).
    pub fn new(secrets: K) -> Result<Self, DeliveryError> {
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent("procurator-ci")
            .build()
            .map_err(DeliveryError::new)?;
        Ok(Self { client, secrets })
    }
}

impl<K: SecretStore> Notifier for WebhookNotifier<K> {
    fn deliver(
        &self,
        target: &Target,
        payload: &EventPayload,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        async move {
            let Target::Webhook(secret) = target;
            let Some(url) = self.secrets.secret(secret).await? else {
                return Err(DeliveryError::new(format!("no secret named {secret:?}")));
            };
            let body = serde_json::to_vec(payload).map_err(DeliveryError::new)?;
            let response = self
                .client
                .post(&url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .await
                // Why: the URL is a secret; keep it out of logs and the DB.
                .map_err(|e| DeliveryError::new(with_causes(&e.without_url())))?;
            let status = response.status();
            if status.is_success() {
                Ok(())
            } else {
                Err(DeliveryError::new(format!("webhook answered {status}")))
            }
        }
    }

    fn retry_delay(&self, _target: &Target, attempts: u32) -> Option<Duration> {
        let index = usize::try_from(attempts).ok()?.checked_sub(1)?;
        BACKOFF.get(index).copied()
    }
}

#[cfg(all(test, feature = "http"))]
mod tests {
    use std::future::Future;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::post;

    use super::WebhookNotifier;
    use crate::application::ports::{DeliveryError, Notifier as _, SecretStore};
    use crate::domain::event::{EventKind, EventPayload, Target};
    use crate::domain::run::{CommitHash, GitRef, NewRun, RepoName, Run, RunId, RunStatus};

    struct Secrets(String);

    impl SecretStore for Secrets {
        fn secret(
            &self,
            name: &str,
        ) -> impl Future<Output = Result<Option<String>, DeliveryError>> + Send {
            let found = (name == "hook").then(|| self.0.clone());
            std::future::ready(Ok(found))
        }
    }

    fn payload() -> EventPayload {
        let request = NewRun::new(
            RepoName::parse(String::from("lucas/demo")).unwrap(),
            CommitHash::parse("a".repeat(40)).unwrap(),
            GitRef::parse(String::from("refs/heads/main")).unwrap(),
            None,
        );
        let run = Run::queued(RunId::new(3), request, 1);
        EventPayload::for_run(EventKind::RunFailed, &run, RunStatus::Failed)
    }

    #[tokio::test]
    async fn posts_the_event_and_reports_failures() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        let app = Router::new()
            .route(
                "/ok",
                post(move |body: String| async move {
                    sink.lock().unwrap().push(body);
                    StatusCode::NO_CONTENT
                }),
            )
            .route("/down", post(|| async { StatusCode::BAD_GATEWAY }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let hook = Target::Webhook(String::from("hook"));
        let ok = WebhookNotifier::new(Secrets(format!("http://{addr}/ok"))).unwrap();
        ok.deliver(&hook, &payload()).await.unwrap();
        let body: serde_json::Value = serde_json::from_str(&received.lock().unwrap()[0]).unwrap();
        assert_eq!(body["kind"], "run.failed");
        assert_eq!(body["repo"], "lucas/demo");
        assert_eq!(body["run"], 3);

        let down = WebhookNotifier::new(Secrets(format!("http://{addr}/down"))).unwrap();
        let error = down.deliver(&hook, &payload()).await.unwrap_err();
        assert!(error.to_string().contains("502"), "{error}");
        let missing = Target::Webhook(String::from("nope"));
        assert!(ok.deliver(&missing, &payload()).await.is_err());
    }

    #[test]
    fn retries_back_off_then_give_up() {
        let notifier = WebhookNotifier::new(Secrets(String::new())).unwrap();
        let hook = Target::Webhook(String::from("hook"));
        let delays: Vec<Option<Duration>> =
            (1..=5).map(|n| notifier.retry_delay(&hook, n)).collect();
        assert_eq!(
            delays,
            [
                Some(Duration::from_secs(30)),
                Some(Duration::from_secs(120)),
                Some(Duration::from_secs(600)),
                Some(Duration::from_secs(1800)),
                None
            ]
        );
    }
}
