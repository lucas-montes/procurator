//! A [`Notifier`] that hands each target to the adapter for its kind.
//! Why: webhooks and deploys are delivered and retried differently, and the
//! deploy adapter differs between the monolith (in process) and a split
//! deployment (HTTP).

use std::future::Future;
use std::time::Duration;

use crate::application::ports::{DeliveryError, Notifier};
use crate::domain::event::{EventPayload, Target};

/// Webhook targets go to `W`, deploy targets to `D`.
#[derive(Debug)]
pub struct Routed<W, D> {
    webhooks: W,
    deploys: D,
}

impl<W: Notifier, D: Notifier> Routed<W, D> {
    /// Route webhooks to `webhooks` and deploys to `deploys`.
    #[must_use]
    pub fn new(webhooks: W, deploys: D) -> Self {
        Self { webhooks, deploys }
    }
}

impl<W: Notifier, D: Notifier> Notifier for Routed<W, D> {
    fn deliver(
        &self,
        target: &Target,
        payload: &EventPayload,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        let to_webhook = matches!(target, Target::Webhook(_));
        async move {
            if to_webhook {
                self.webhooks.deliver(target, payload).await
            } else {
                self.deploys.deliver(target, payload).await
            }
        }
    }

    fn retry_delay(&self, target: &Target, attempts: u32) -> Option<Duration> {
        match target {
            Target::Webhook(_) => self.webhooks.retry_delay(target, attempts),
            Target::Deploy(_) => self.deploys.retry_delay(target, attempts),
        }
    }
}
