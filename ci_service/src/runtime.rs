//! The CI service's production wiring: adapters chosen from a [`Config`],
//! background tasks started, HTTP router built.
//! Why: the standalone binary and the monolith run the same service; only
//! the [`Access`] adapter differs.

use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use tokio::task::JoinSet;

use crate::adapters::http::router;
use crate::adapters::nix::{GitConfigSource, NixBuilder};
use crate::adapters::notify::Routed;
use crate::adapters::secrets::FileSecretStore;
use crate::adapters::sqlite::SqliteRunStore;
use crate::adapters::system::SystemClock;
use crate::adapters::webhook::WebhookNotifier;
use crate::application::ports::{Access, DeliveryError, Notifier, StoreError};
use crate::application::{Service, ServiceError};
use crate::config::Config;
use crate::domain::repo_config::{CachePolicy, Limits, Retention};

/// How often expired logs are deleted.
const RETENTION_SWEEP: Duration = Duration::from_secs(3600);

/// Why the service could not start.
#[derive(Debug)]
pub enum StartError {
    /// The run store could not be opened.
    Store(StoreError),
    /// Interrupted runs could not be requeued.
    Recover(ServiceError),
    /// The webhook client could not be built.
    Notifier(DeliveryError),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(e) => e.fmt(f),
            Self::Recover(e) => write!(f, "recover interrupted runs: {e}"),
            Self::Notifier(e) => write!(f, "webhook client: {e}"),
        }
    }
}

impl std::error::Error for StartError {}

/// A started CI service: its workers, dispatcher and retention task run
/// until this is dropped.
#[derive(Debug)]
pub struct Runtime {
    router: Router,
    bind_address: String,
    /// Why: dropping the set aborts the tasks; a run cut off this way is
    /// requeued at the next start.
    #[expect(
        dead_code,
        reason = "held only so dropping the runtime stops its tasks"
    )]
    tasks: JoinSet<()>,
}

impl Runtime {
    /// Open the store, requeue interrupted runs and start
    /// `max_concurrent_runs` workers, the outbox dispatcher and log
    /// retention. Reads of runs and logs are checked with `access`, and
    /// deploys are delivered by `deploys` (e.g. [`HttpDeploy`]).
    ///
    /// [`HttpDeploy`]: crate::adapters::webhook::HttpDeploy
    ///
    /// # Errors
    ///
    /// [`StartError`] when the store, the recovery or the webhook client
    /// fails.
    pub fn start<A: Access, D: Notifier>(
        config: Config,
        access: A,
        deploys: D,
    ) -> impl Future<Output = Result<Self, StartError>> + Send {
        async move {
            let store = SqliteRunStore::open(&config.database_url)
                .await
                .map_err(StartError::Store)?;
            let repos = PathBuf::from(config.repos_base_path);
            let mut builder =
                NixBuilder::new(repos.clone(), config.eval_workers, &config.allowed_uris);
            let secrets = FileSecretStore::new(config.secrets_file.map(PathBuf::from));
            let webhooks = WebhookNotifier::new(secrets).map_err(StartError::Notifier)?;
            let notifier = Routed::new(webhooks, deploys);
            let limits = Limits::new(
                config.max_reruns,
                config.max_check_timeout,
                config.max_run_timeout,
            );
            let retention = Retention::new(
                config.default_log_retention_days,
                config.max_log_retention_days,
            );
            let mut cache = None;
            if let Some(base) = &config.cache_base_url {
                builder = builder.with_cache(base, config.netrc_file.map(PathBuf::from));
                cache = Some(CachePolicy::new(
                    config.public_cache,
                    config.private_cache_prefix,
                ));
            }
            let mut service = Service::new(
                store,
                SystemClock,
                builder,
                GitConfigSource::new(repos),
                notifier,
                access,
            )
            .with_limits(limits)
            .with_retention(retention);
            if let Some(policy) = cache {
                service = service.with_cache(policy);
            }
            let service = Arc::new(service);

            // Why: no worker is running yet, so every `running` row is a
            // leftover.
            let recovered = service.recover().await.map_err(StartError::Recover)?;
            if recovered > 0 {
                tracing::warn!(
                    recovered,
                    "requeued runs interrupted by the previous process"
                );
            }

            let mut tasks = JoinSet::new();
            let poll = Duration::from_millis(config.worker_poll_interval_ms);
            for _ in 0..config.max_concurrent_runs {
                let service = Arc::clone(&service);
                tasks.spawn(async move { service.work(poll).await });
            }
            // Why: one dispatcher; deliveries survive a restart in the outbox.
            let dispatcher = Arc::clone(&service);
            tasks.spawn(async move { dispatcher.dispatch(poll).await });
            let retention = Arc::clone(&service);
            tasks.spawn(async move { retention.expire_logs(RETENTION_SWEEP).await });
            tracing::info!(workers = config.max_concurrent_runs, "CI service started");
            Ok(Self {
                router: router(service),
                bind_address: config.bind_address,
                tasks,
            })
        }
    }

    /// The HTTP API. Serve it, or merge it into a larger router.
    pub fn router(&self) -> Router {
        self.router.clone()
    }

    /// Where the config says to serve the API.
    #[must_use]
    pub fn bind_address(&self) -> &str {
        &self.bind_address
    }
}
