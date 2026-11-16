//! Deleting old logs. Why: logs are the bulk of the store; runs and checks
//! stay, so history and flakiness survive.

use std::future::Future;
use std::time::Duration;

use super::Service;
use crate::application::ServiceError;
use crate::application::ports::{Access, Builder, Clock, ConfigSource, Notifier, RunStore};

impl<S, C, B, R, N, A> Service<S, C, B, R, N, A>
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
    N: Notifier,
    A: Access,
{
    /// Delete expired logs now, then every `every`, forever. Drop the
    /// future to stop.
    pub fn expire_logs(&self, every: Duration) -> impl Future<Output = ()> + Send + '_ {
        async move {
            loop {
                match self.delete_expired_logs().await {
                    Ok(0) => {}
                    Ok(lines) => tracing::info!(lines, "deleted expired log lines"),
                    Err(error) => tracing::error!(%error, "log retention failed"),
                }
                tokio::time::sleep(every).await;
            }
        }
    }

    /// Delete the logs of every run kept long enough. Returns how many
    /// lines.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Internal`] when the store fails.
    pub fn delete_expired_logs(
        &self,
    ) -> impl Future<Output = Result<u64, ServiceError>> + Send + '_ {
        async move { Ok(self.store.delete_expired_logs(self.clock.now()).await?) }
    }
}
