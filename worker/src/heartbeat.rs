//! Worker heartbeat reporter: periodic `pushData` from registry contents.
//!
//! Mapping mirrors the `listVms`/`read` RPC shaping exactly: per-VM status is
//! `running`/`failed` from `health()`, worker health is the conjunction, and
//! everything the worker does not track is zero-filled like `read()` does:
//! `contentHash` is `""`, uptimes are `0`, and all resource/metric fields are
//! `0`. The observed generation comes from the poll loop's shared cell (0
//! until the first poll lands — liveness first, the control plane correctly
//! counts such workers as not-up-to-date).
//!
//! Like the poll loop this is thread-local (capability clients are not
//! `Send`), so [`PushClient`] carries no `Send`/`Sync` bounds.

use std::time::Duration;

use tokio::time::sleep;

use super::{
    pull::{AppliedGeneration, PollConfig, RpcPullClient, TransportLost, backoff_delay},
    vmm::{Factory, Handle, Reader, Registry},
};

/// One running VM as observed locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningVmView {
    /// Registry id.
    pub id: String,
    /// Whether its last health check passed.
    pub healthy: bool,
}

/// Heartbeat payload pushed to the control plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatReport {
    /// This worker's id.
    pub worker_id: String,
    /// Last generation the poll loop applied (0 when none yet).
    pub observed_generation: u64,
    /// Currently registered VMs, sorted by id for stable payloads.
    pub running: Vec<RunningVmView>,
    /// Conjunction of all VM health checks.
    pub healthy: bool,
}

/// Push failures: broken transport (redial) vs server rejection (back off,
/// keep the connection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushError {
    /// Dial, framing, or connection loss.
    Transport(String),
    /// The node rejected the heartbeat (mapped from the `Result` union).
    Rejected(String),
}

impl std::fmt::Display for PushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(reason) => write!(f, "master transport failed: {reason}"),
            Self::Rejected(message) => write!(f, "master rejected heartbeat: {message}"),
        }
    }
}

impl std::error::Error for PushError {}

/// Heartbeat sink pushed by [`heartbeat_loop`]. Mocked in tests; the
/// production impl is [`RpcPullClient`] (see `pull.rs`).
pub trait PushClient {
    /// Pushes one heartbeat report.
    ///
    /// # Errors
    ///
    /// Returns [`PushError`] on transport loss or server rejection.
    fn push(&self, report: &HeartbeatReport) -> impl Future<Output = Result<(), PushError>>;
}

/// Builds a heartbeat report from current registry contents.
pub async fn build_report<F: Factory>(
    worker_id: &str,
    applied: Option<u64>,
    registry: &Registry<F, Reader>,
) -> HeartbeatReport {
    let guard = registry.clone().get().await;
    let mut running = Vec::with_capacity(guard.len());
    let mut healthy = true;
    for (id, handle) in guard.iter() {
        let vm_healthy = handle.health().await.is_ok();
        healthy = healthy && vm_healthy;
        running.push(RunningVmView {
            id: id.clone(),
            healthy: vm_healthy,
        });
    }
    running.sort_by(|left, right| left.id.cmp(&right.id));
    HeartbeatReport {
        worker_id: worker_id.to_owned(),
        observed_generation: applied.unwrap_or(0),
        running,
        healthy,
    }
}

/// Pushes heartbeats until the connection breaks: builds from the registry,
/// sends, and backs off on rejections. A missed control plane never crashes
/// anything — errors stay inside this loop.
pub async fn heartbeat_loop<S, F>(
    sink: &S,
    worker_id: &str,
    interval: Duration,
    applied: &AppliedGeneration,
    registry: &Registry<F, Reader>,
) -> TransportLost
where
    S: PushClient,
    F: Factory,
{
    let mut failures: u32 = 0;
    loop {
        let report = build_report(worker_id, applied.get(), registry).await;
        match sink.push(&report).await {
            Ok(()) => {
                failures = 0;
                tracing::debug!(
                    worker = worker_id,
                    generation = report.observed_generation,
                    vms = report.running.len(),
                    "Heartbeat pushed"
                );
            }
            Err(PushError::Rejected(message)) => {
                failures += 1;
                tracing::warn!(worker = worker_id, %message, "Heartbeat rejected");
                sleep(backoff_delay(failures, interval)).await;
                continue;
            }
            Err(PushError::Transport(reason)) => {
                tracing::warn!(worker = worker_id, %reason, "Master connection lost");
                return TransportLost;
            }
        }
        sleep(interval).await;
    }
}

/// Dials the control plane and heartbeats forever, redialing with backoff
/// after dial failures and pausing one interval after transport breaks.
pub async fn run_heartbeat_loop<F>(
    config: PollConfig,
    registry: Registry<F, Reader>,
    applied: AppliedGeneration,
) where
    F: Factory,
    F::BackendConfig: Unpin,
{
    let mut failures: u32 = 0;
    loop {
        match RpcPullClient::<F::BackendConfig>::connect(config.master_addr).await {
            Ok(client) => {
                failures = 0;
                heartbeat_loop(
                    &client,
                    &config.worker_id,
                    config.heartbeat_interval,
                    &applied,
                    &registry,
                )
                .await;
                sleep(config.heartbeat_interval).await;
            }
            Err(err) => {
                failures += 1;
                tracing::warn!(
                    master = %config.master_addr,
                    %err,
                    failures,
                    "Master unreachable"
                );
                sleep(backoff_delay(failures, config.heartbeat_interval)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use capnp::message::ReaderOptions;
    use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
    use futures::AsyncReadExt;
    use tokio_util::compat::TokioAsyncReadCompatExt;

    use super::{HeartbeatReport, PushClient, PushError, build_report, heartbeat_loop};
    use crate::{
        pull::AppliedGeneration,
        test_support::{FakeFactory, FakeHandle, TempDb},
        vmm::{Reader, Registry, Writer},
    };

    async fn test_registry(names: &[(&str, bool)]) -> (Registry<FakeFactory, Reader>, TempDb) {
        let (db, guard) = TempDb::create("heartbeat").await;
        let registry = Registry::<FakeFactory, _>::new(db);
        let (reader, mut writer): (Registry<FakeFactory, Reader>, Registry<FakeFactory, Writer>) =
            registry.split();
        for (name, healthy) in names {
            let handle = if *healthy {
                FakeHandle::healthy()
            } else {
                FakeHandle::failing()
            };
            writer.insert((*name).to_string(), handle).await;
        }
        (reader, guard)
    }

    #[derive(Debug, Default)]
    struct MockSink {
        pushes: Mutex<Vec<HeartbeatReport>>,
        script: Mutex<VecDeque<Result<(), PushError>>>,
    }

    impl MockSink {
        fn scripted(script: Vec<Result<(), PushError>>) -> Self {
            Self {
                pushes: Mutex::new(Vec::new()),
                script: Mutex::new(script.into()),
            }
        }

        fn pushes(&self) -> Vec<HeartbeatReport> {
            self.pushes.lock().expect("pushes").clone()
        }
    }

    impl PushClient for MockSink {
        async fn push(&self, report: &HeartbeatReport) -> Result<(), PushError> {
            self.pushes.lock().expect("pushes").push(report.clone());
            let mut script = self.script.lock().expect("script");
            script.pop_front().unwrap_or(Ok(()))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn build_report_shapes_registry_contents() {
        let (reader, _db) = test_registry(&[("ok-vm", true), ("bad-vm", false)]).await;

        let applied = AppliedGeneration::new();
        applied.set(9);
        let report = build_report::<FakeFactory>("w1", applied.get(), &reader).await;
        assert_eq!(report.worker_id, "w1");
        assert_eq!(report.observed_generation, 9);
        assert!(!report.healthy);
        assert_eq!(
            report
                .running
                .iter()
                .map(|vm| (vm.id.as_str(), vm.healthy))
                .collect::<Vec<_>>(),
            vec![("bad-vm", false), ("ok-vm", true)]
        );

        let fresh =
            build_report::<FakeFactory>("w1", AppliedGeneration::new().get(), &reader).await;
        assert_eq!(fresh.observed_generation, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn heartbeat_loop_retries_then_succeeds() {
        let (reader, _db) = test_registry(&[("vm-a", true)]).await;
        let applied = AppliedGeneration::new();
        applied.set(3);
        let sink = MockSink::scripted(vec![
            Err(PushError::Rejected("busy-1".to_string())),
            Err(PushError::Rejected("busy-2".to_string())),
            Ok(()),
        ]);

        // Rejections back off and retry; the loop itself never returns.
        let outcome = tokio::time::timeout(
            Duration::from_millis(300),
            heartbeat_loop(&sink, "w1", Duration::from_millis(5), &applied, &reader),
        )
        .await;
        assert!(outcome.is_err(), "loop must not return on its own");

        let pushes = sink.pushes();
        assert!(pushes.len() >= 3, "unexpected pushes: {pushes:?}");
        assert!(
            pushes
                .iter()
                .all(|push| push.observed_generation == 3 && push.running.len() == 1),
            "unexpected pushes: {pushes:?}"
        );
        let delivered = pushes.last().expect("at least one push");
        assert_eq!(delivered.worker_id, "w1");
        assert!(delivered.healthy);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn heartbeat_loop_returns_on_transport_loss() {
        use crate::pull::TransportLost;

        let (reader, _db) = test_registry(&[("vm-a", true)]).await;
        let applied = AppliedGeneration::new();
        let sink = MockSink::scripted(vec![Err(PushError::Transport("down".to_string()))]);

        let result = heartbeat_loop(&sink, "w1", Duration::from_millis(5), &applied, &reader).await;
        assert_eq!(result, TransportLost);
        assert_eq!(sink.pushes().len(), 1);
    }

    #[derive(Clone, Default)]
    struct CapturedPush {
        worker_id: String,
        generation: u64,
        vms: Vec<(String, String)>,
    }

    #[derive(Clone)]
    struct StubMaster {
        pushes: Arc<Mutex<Vec<CapturedPush>>>,
        reject: Arc<Mutex<Option<String>>>,
    }

    impl commands::master_capnp::master::Server<capnp::any_pointer::Owned> for StubMaster {
        fn publish_state(
            &mut self,
            _: commands::master_capnp::master::PublishStateParams<capnp::any_pointer::Owned>,
            _: commands::master_capnp::master::PublishStateResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            capnp::capability::Promise::err(capnp::Error::failed("stub".to_string()))
        }

        fn get_assignment(
            &mut self,
            _: commands::master_capnp::master::GetAssignmentParams<capnp::any_pointer::Owned>,
            _: commands::master_capnp::master::GetAssignmentResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            capnp::capability::Promise::err(capnp::Error::failed("stub".to_string()))
        }

        fn push_data(
            &mut self,
            params: commands::master_capnp::master::PushDataParams<capnp::any_pointer::Owned>,
            mut results: commands::master_capnp::master::PushDataResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            let pushes = self.pushes.clone();
            let reject = self.reject.clone();
            capnp::capability::Promise::from_future(async move {
                let params = params.get()?;
                let worker_id = params
                    .get_worker_id()?
                    .to_str()
                    .map_err(|err| capnp::Error::failed(format!("utf-8: {err}")))?
                    .to_owned();
                let generation = params.get_observed_generation();
                let wire_vms = params.get_running_vms()?;
                let mut vms = Vec::new();
                for vm in wire_vms {
                    vms.push((
                        vm.get_id()?
                            .to_str()
                            .map_err(|err| capnp::Error::failed(format!("utf-8: {err}")))?
                            .to_owned(),
                        vm.get_status()?
                            .to_str()
                            .map_err(|err| capnp::Error::failed(format!("utf-8: {err}")))?
                            .to_owned(),
                    ));
                }
                pushes.lock().expect("pushes").push(CapturedPush {
                    worker_id,
                    generation,
                    vms,
                });
                if let Some(message) = reject.lock().expect("reject").clone() {
                    results.get().get_result()?.set_err(message.as_str())?;
                } else {
                    let _ = results.get().get_result()?.init_ok();
                }
                Ok(())
            })
        }

        fn get_cluster_status(
            &mut self,
            _: commands::master_capnp::master::GetClusterStatusParams<capnp::any_pointer::Owned>,
            _: commands::master_capnp::master::GetClusterStatusResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            capnp::capability::Promise::err(capnp::Error::failed("stub".to_string()))
        }

        fn get_worker(
            &mut self,
            _: commands::master_capnp::master::GetWorkerParams<capnp::any_pointer::Owned>,
            _: commands::master_capnp::master::GetWorkerResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            capnp::capability::Promise::err(capnp::Error::failed("stub".to_string()))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rpc_push_roundtrip_maps_registry_shape() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let pushes = Arc::new(Mutex::new(Vec::new()));
                let reject = Arc::new(Mutex::new(None));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("bind");
                let addr = listener.local_addr().expect("addr");
                let stub = StubMaster {
                    pushes: pushes.clone(),
                    reject: reject.clone(),
                };
                let _serve_task = tokio::task::spawn_local(async move {
                    let (stream, _) = listener.accept().await.expect("accept");
                    stream.set_nodelay(true).expect("nodelay");
                    let (reader, writer) = TokioAsyncReadCompatExt::compat(stream).split();
                    let network = twoparty::VatNetwork::new(
                        futures::io::BufReader::new(reader),
                        futures::io::BufWriter::new(writer),
                        rpc_twoparty_capnp::Side::Server,
                        ReaderOptions::default(),
                    );
                    let client: commands::master_capnp::master::Client<capnp::any_pointer::Owned> =
                        capnp_rpc::new_client(stub);
                    RpcSystem::new(Box::new(network), Some(client.client))
                        .await
                        .expect("rpc system runs");
                });

                let client = crate::pull::RpcPullClient::<capnp::any_pointer::Owned>::connect(addr)
                    .await
                    .expect("connect");
                let (reader, _db) = test_registry(&[("ok-vm", true), ("bad-vm", false)]).await;
                let report = build_report::<FakeFactory>("w1", Some(4), &reader).await;
                PushClient::push(&client, &report)
                    .await
                    .expect("push succeeds");

                let captured = pushes.lock().expect("pushes").clone();
                assert_eq!(captured.len(), 1);
                assert_eq!(captured[0].worker_id, "w1");
                assert_eq!(captured[0].generation, 4);
                assert_eq!(
                    captured[0].vms,
                    vec![
                        ("bad-vm".to_string(), "failed".to_string()),
                        ("ok-vm".to_string(), "running".to_string()),
                    ]
                );

                *reject.lock().expect("reject") = Some("node busy".to_string());
                match PushClient::push(&client, &report).await {
                    Err(PushError::Rejected(message)) => {
                        assert!(message.contains("node busy"), "unexpected: {message}");
                    }
                    other => panic!("expected rejection, got {other:?}"),
                }
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_heartbeat_loop_pushes_without_a_master() {
        // No control plane listens here: the loop must back off forever
        // without crashing or pushing. A short timeout bounds the run.
        let (reader, _db) = test_registry(&[("vm-a", true)]).await;
        let applied = AppliedGeneration::new();
        let outcome = tokio::time::timeout(
            Duration::from_millis(200),
            super::run_heartbeat_loop::<FakeFactory>(
                crate::pull::PollConfig {
                    worker_id: "w1".to_string(),
                    master_addr: "127.0.0.1:1".parse().expect("addr"),
                    poll_interval: Duration::from_millis(5),
                    heartbeat_interval: Duration::from_millis(5),
                },
                reader,
                applied,
            ),
        )
        .await;
        assert!(outcome.is_err(), "loop must not return on its own");
    }
}
