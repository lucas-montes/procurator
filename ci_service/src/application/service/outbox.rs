//! The dispatcher: sends queued deliveries and retries them on each
//! notifier's terms.
//! Why: deliveries are written with the run's result, so a crash between
//! finishing and notifying loses nothing; a down webhook only delays its own
//! deliveries.

use std::future::Future;
use std::time::Duration;

use super::Service;
use crate::application::ServiceError;
use crate::application::ports::{
    Access, Builder, Clock, ConfigSource, NextAttempt, Notifier, RunStore,
};

/// Deliveries attempted per pass.
const BATCH: u32 = 50;

impl<S, C, B, R, N, A> Service<S, C, B, R, N, A>
where
    S: RunStore,
    C: Clock,
    B: Builder,
    R: ConfigSource,
    N: Notifier,
    A: Access,
{
    /// Deliver due events forever, waiting `poll` when none is due. Drop the
    /// future to stop; pending deliveries stay queued.
    pub fn dispatch(&self, poll: Duration) -> impl Future<Output = ()> + Send + '_ {
        async move {
            loop {
                match self.deliver_due().await {
                    Ok(0) => tokio::time::sleep(poll).await,
                    Ok(_) => {}
                    Err(error) => {
                        tracing::error!(%error, "dispatcher failed");
                        tokio::time::sleep(poll).await;
                    }
                }
            }
        }
    }

    /// Attempt every delivery due now, once. Returns how many were tried.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Internal`] when the store fails.
    pub fn deliver_due(&self) -> impl Future<Output = Result<usize, ServiceError>> + Send + '_ {
        async move {
            let due = self.store.due_deliveries(self.clock.now(), BATCH).await?;
            for delivery in &due {
                let result = self
                    .notifier
                    .deliver(delivery.target(), delivery.payload())
                    .await;
                let (next, error) = match result {
                    Ok(()) => (NextAttempt::Delivered, None),
                    Err(error) => {
                        let failed = delivery.attempts() + 1;
                        let next = match self.notifier.retry_delay(delivery.target(), failed) {
                            Some(delay) => {
                                let delay = i64::try_from(delay.as_secs()).unwrap_or(i64::MAX);
                                NextAttempt::RetryAt(self.clock.now().saturating_add(delay))
                            }
                            None => NextAttempt::GiveUp,
                        };
                        tracing::warn!(
                            delivery = delivery.id(),
                            run = delivery.payload().run(),
                            attempts = failed,
                            %error,
                            "delivery failed"
                        );
                        (next, Some(error.to_string()))
                    }
                };
                self.store
                    .record_attempt(delivery.id(), next, error.as_deref())
                    .await?;
            }
            Ok(due.len())
        }
    }
}
