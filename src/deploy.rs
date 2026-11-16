//! The CI's deploy target in the monolith: the embedded control plane,
//! called directly instead of over HTTP.
//! Why: `ci_service` and `control_plane` never depend on each other; this
//! composition root is the only place that knows both.

use std::future::Future;
use std::time::Duration;

use ci_service::application::ports::{DeliveryError, Notifier};
use ci_service::domain::event::{EventPayload, Target};
use control_plane::{DesiredVm, Publisher, VmImage};

/// Bytes per MiB.
const MIB: u64 = 1024 * 1024;
/// Waits between deploy attempts; the last repeats forever.
/// Why: a lost deploy means the cluster never converges.
const BACKOFF: [Duration; 4] = [
    Duration::from_secs(30),
    Duration::from_secs(120),
    Duration::from_secs(600),
    Duration::from_secs(1800),
];

/// Publishes each `run.succeeded` deployment as the control plane's next
/// generation. `deploy.url` is not used.
#[derive(Debug)]
pub struct ControlPlaneDeploy {
    publisher: Publisher,
}

impl ControlPlaneDeploy {
    /// Deploy through `publisher`.
    pub fn new(publisher: Publisher) -> Self {
        Self { publisher }
    }
}

impl Notifier for ControlPlaneDeploy {
    fn deliver(
        &self,
        _target: &Target,
        payload: &EventPayload,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        async move {
            let Some(deployment) = payload.deployment() else {
                return Err(DeliveryError::new("no deployment in the event"));
            };
            let vms = deployment
                .vms()
                .iter()
                .map(|vm| {
                    let image = VmImage::new(vm.kernel(), vm.initramfs(), vm.cmdline(), vm.disk())
                        .with_resources(vm.cpus(), vm.memory_mib().saturating_mul(MIB));
                    DesiredVm::new(vm.id(), vm.worker(), &image)
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(DeliveryError::new)?;
            let generation = self
                .publisher
                .deploy(payload.commit(), deployment.path(), vms)
                .await
                .map_err(DeliveryError::new)?;
            tracing::info!(
                run = payload.run(),
                generation,
                vms = deployment.vms().len(),
                "deployed"
            );
            Ok(())
        }
    }

    fn retry_delay(&self, _target: &Target, attempts: u32) -> Option<Duration> {
        let last = BACKOFF.len() - 1;
        let index = usize::try_from(attempts.saturating_sub(1)).map_or(last, |i| i.min(last));
        BACKOFF.get(index).copied()
    }
}
