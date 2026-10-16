//! Worker pull loop: poll the control-plane assignment, reconcile the registry.
//!
//! The loop is thread-local like the RPC server (capability clients are not
//! `Send`), so [`PullClient`] carries no `Send`/`Sync` bounds and the task
//! runs on the crate `LocalSet`.
//!
//! Identity (plan T09): each desired `VmSpec` carries an `id` used as the
//! reconcile key; blank ids are skipped with a warning (fail-safe: never
//! manage what cannot be identified). Spec bytes keep their wire brand and
//! are reinterpreted as the backend struct at materialize time (identical
//! struct layout, same deployment backend). The [`Reconciler`] itself stays
//! key-opaque.
//! Specs cross the client boundary as serialized bytes so polls stay
//! message-scoped (no borrowed readers escape the poll iteration).

use std::{collections::HashSet, net::SocketAddr, time::Duration};

use capnp::message::ReaderOptions;
use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use futures::AsyncReadExt;
use tokio::{sync::mpsc::Sender, time::sleep};
use tokio_util::compat::TokioAsyncReadCompatExt;

use super::{
    heartbeat::{HeartbeatReport, PushClient, PushError},
    vmm::{Command, CreateCommand, Error as VmmError, Factory, Reader, Registry},
};

/// Upper bound for poll backoff between retries.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Last generation the poll loop applied, shared with the heartbeat task so
/// `pushData` reports what this worker actually runs. Starts empty (the
/// heartbeat reports generation 0 until the first poll lands).
#[derive(Debug, Clone, Default)]
pub struct AppliedGeneration(std::sync::Arc<std::sync::Mutex<Option<u64>>>);

impl AppliedGeneration {
    /// Creates an empty (nothing applied yet) cell.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the last applied generation, if any.
    pub fn get(&self) -> Option<u64> {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Records a newly applied generation.
    pub fn set(&self, generation: u64) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(generation);
    }
}

/// Poll task parameters: identity, control-plane route, and cadence. Grows
/// here (heartbeat interval lives alongside poll cadence) so the runtime
/// keeps its shape.
#[derive(Debug, Clone)]
pub struct PollConfig {
    /// This worker's id, polled as.
    pub worker_id: String,
    /// Control-plane address dialed for assignments.
    pub master_addr: SocketAddr,
    /// Interval between assignment polls.
    pub poll_interval: Duration,
    /// Interval between heartbeat pushes.
    pub heartbeat_interval: Duration,
}

/// One desired VM: stable key plus opaque spec bytes materialized only for
/// missing keys at reconcile time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredVm {
    /// Stable identity (spec index until plan T09 adds real ids).
    pub key: String,
    /// Serialized `VmSpec` bytes, deserialized only when creating.
    pub spec: Vec<u8>,
}

/// Owned assignment snapshot from one poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PulledAssignment {
    /// Control-plane generation this snapshot belongs to.
    pub generation: u64,
    /// Desired VMs for this worker.
    pub desired: Vec<DesiredVm>,
}

/// Pull failures: broken transport (reconnect) vs server rejection (back off,
/// keep the connection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullError {
    /// Dial, framing, or connection loss.
    Transport(String),
    /// The node rejected the request (mapped from the `Result` union).
    Rejected(String),
}

impl std::fmt::Display for PullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(reason) => write!(f, "master transport failed: {reason}"),
            Self::Rejected(message) => write!(f, "master rejected poll: {message}"),
        }
    }
}

impl std::error::Error for PullError {}

/// Assignment source polled by [`poll_loop`]. Mocked in tests; [`RpcPullClient`]
/// serves production traffic.
pub trait PullClient {
    /// Fetches this worker's assignment snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`PullError`] on transport loss or server rejection.
    fn fetch(
        &self,
        worker_id: &str,
        last_seen_generation: u64,
    ) -> impl Future<Output = Result<PulledAssignment, PullError>>;
}

/// Cap'n Proto `Master` assignment client.
pub struct RpcPullClient<T: capnp::traits::Owned> {
    client: commands::master_capnp::master::Client<T>,
}

impl<T: capnp::traits::Owned + 'static> RpcPullClient<T> {
    /// Dials the control plane, spawning the RPC driver on the caller's
    /// `LocalSet` (the driver future is thread-local).
    ///
    /// # Errors
    ///
    /// Returns [`PullError::Transport`] when dialing fails.
    pub async fn connect(master_addr: SocketAddr) -> Result<Self, PullError> {
        let refused =
            |err: std::io::Error| PullError::Transport(format!("dial {master_addr}: {err}"));
        let stream = tokio::net::TcpStream::connect(master_addr)
            .await
            .map_err(refused)?;
        stream.set_nodelay(true).map_err(refused)?;
        let (reader, writer) = TokioAsyncReadCompatExt::compat(stream).split();
        let network = twoparty::VatNetwork::new(
            futures::io::BufReader::new(reader),
            futures::io::BufWriter::new(writer),
            rpc_twoparty_capnp::Side::Client,
            ReaderOptions::default(),
        );
        let mut rpc_system = RpcSystem::new(Box::new(network), None);
        let client = rpc_system.bootstrap(rpc_twoparty_capnp::Side::Server);
        tokio::task::spawn_local(rpc_system);
        Ok(Self { client })
    }
}

impl<T: capnp::traits::Owned + 'static + Unpin> PullClient for RpcPullClient<T> {
    async fn fetch(
        &self,
        worker_id: &str,
        last_seen_generation: u64,
    ) -> Result<PulledAssignment, PullError> {
        let mut request = self.client.get_assignment_request();
        {
            let mut params = request.get();
            params.set_worker_id(worker_id);
            params.set_last_seen_generation(last_seen_generation);
        }
        let transport = |err: capnp::Error| PullError::Transport(format!("getAssignment: {err}"));
        let response = request.send().promise.await.map_err(transport)?;
        let result = response.get().map_err(transport)?;
        let result = result.get_result().map_err(transport)?;
        let not_in_schema =
            |_| PullError::Transport("getAssignment: response not in schema".to_string());
        match result.which().map_err(not_in_schema)? {
            commands::common_capnp::result::Which::Ok(assignment) => {
                let assignment = assignment.map_err(transport)?;
                let generation = assignment.get_generation();
                let specs = assignment.get_desired_vms().map_err(transport)?;
                let mut desired = Vec::new();
                for spec in specs {
                    let raw_id = spec
                        .get_id()
                        .map_err(transport)?
                        .to_str()
                        .map_err(|err| PullError::Transport(format!("getAssignment id: {err}")))?
                        .to_owned();
                    if raw_id.trim().is_empty() {
                        tracing::warn!(
                            "Skipping desiredVm with blank id (publishers must set VmSpec.id)"
                        );
                        continue;
                    }
                    // Response specs stay in their wire brand; materialize
                    // reinterprets the identical layout as the backend struct.
                    desired.push(DesiredVm {
                        key: raw_id,
                        spec: serialize_spec(spec).map_err(|err| {
                            PullError::Transport(format!("serialize spec: {err}"))
                        })?,
                    });
                }
                Ok(PulledAssignment {
                    generation,
                    desired,
                })
            }
            commands::common_capnp::result::Which::Err(text) => {
                let message = text
                    .map_err(transport)?
                    .to_str()
                    .map_err(|err| PullError::Transport(format!("getAssignment text: {err}")))?
                    .to_owned();
                Err(PullError::Rejected(message))
            }
        }
    }
}

impl<T: capnp::traits::Owned + 'static + Unpin> PushClient for RpcPullClient<T> {
    async fn push(&self, report: &HeartbeatReport) -> Result<(), PushError> {
        let mut request = self.client.push_data_request();
        {
            let mut params = request.get();
            params.set_worker_id(report.worker_id.as_str());
            params.set_observed_generation(report.observed_generation);
            {
                let count = u32::try_from(report.running.len()).unwrap_or(u32::MAX);
                let mut running = params.reborrow().init_running_vms(count);
                for (index, vm) in report.running.iter().enumerate() {
                    let index = u32::try_from(index).unwrap_or(u32::MAX);
                    let mut wire = running.reborrow().get(index);
                    wire.set_id(vm.id.as_str());
                    // Untracked until later work (mirrors listVms/read
                    // zero-filling): no content hashes or uptimes anywhere.
                    wire.set_content_hash("");
                    wire.set_status(if vm.healthy { "running" } else { "failed" });
                    wire.set_uptime(0);
                    let mut metrics = wire.reborrow().init_metrics();
                    metrics.set_cpu_usage(0.0);
                    metrics.set_memory_usage(0);
                    metrics.set_network_rx_bytes(0);
                    metrics.set_network_tx_bytes(0);
                }
            }
            {
                let mut metrics = params.reborrow().init_metrics();
                metrics.set_available_cpu(0.0);
                metrics.set_available_memory(0);
                metrics.set_disk_usage(0);
                metrics.set_uptime(0);
            }
        }
        let transport = |err: capnp::Error| PushError::Transport(format!("pushData: {err}"));
        let response = request.send().promise.await.map_err(transport)?;
        let result = response.get().map_err(transport)?;
        let result = result.get_result().map_err(transport)?;
        let not_in_schema =
            |_| PushError::Transport("pushData: response not in schema".to_string());
        match result.which().map_err(not_in_schema)? {
            commands::common_capnp::result::Which::Ok(_) => Ok(()),
            commands::common_capnp::result::Which::Err(text) => {
                let message = text
                    .map_err(transport)?
                    .to_str()
                    .map_err(|err| PushError::Transport(format!("pushData text: {err}")))?
                    .to_owned();
                Err(PushError::Rejected(message))
            }
        }
    }
}

/// Serializes one borrowed spec into owned bytes so polls stay
/// message-scoped; deserialized iteration-locally at reconcile time.
fn serialize_spec<T: capnp::traits::Owned>(
    spec: commands::common_capnp::vm_spec::Reader<'_, T>,
) -> Result<Vec<u8>, capnp::Error> {
    let mut message = capnp::message::Builder::new_default();
    message.set_root(spec)?;
    let mut bytes = Vec::new();
    capnp::serialize::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

/// Reconcile outcome: ids created and deleted, in command order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileOutcome {
    /// Keys a `Create` was enqueued for, in desired order.
    pub created: Vec<String>,
    /// Ids a `Delete` was enqueued for, sorted.
    pub deleted: Vec<String>,
}

/// Reconcile failures.
#[derive(Debug)]
pub enum ReconcileError {
    /// Spec bytes failed to decode or convert.
    Spec(String),
    /// The backend factory failed to build a creation command.
    Factory(VmmError),
    /// The supervisor is gone (command channel closed).
    ChannelClosed,
}

impl std::fmt::Display for ReconcileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spec(message) => write!(f, "bad desired spec: {message}"),
            Self::Factory(err) => write!(f, "factory failed: {err}"),
            Self::ChannelClosed => write!(f, "supervisor command channel is closed"),
        }
    }
}

impl std::error::Error for ReconcileError {}

/// Diffs desired keys against the registry and drives the supervisor channel.
/// Creates materialize specs iteration-locally (bytes never outlive the call);
/// deletes and no-ops need no specs at all.
#[derive(Debug, Clone)]
pub struct Reconciler<F: Factory> {
    factory: F,
    tx: Sender<Command<F>>,
}

impl<F: Factory> Reconciler<F> {
    /// Builds a reconciler over `factory` emitting into the supervisor channel.
    pub fn new(factory: F, tx: Sender<Command<F>>) -> Self {
        Self { factory, tx }
    }

    /// Reconciles `desired` against `registry`: creates missing keys, deletes
    /// extras, and stays silent otherwise. Never touches up-to-date VMs (no
    /// spurious restarts).
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError`] on spec decode failure, factory failure, or
    /// a dead supervisor channel. Partial progress may already be enqueued.
    pub async fn reconcile(
        &self,
        desired: &[DesiredVm],
        registry: &Registry<F, Reader>,
    ) -> Result<ReconcileOutcome, ReconcileError> {
        let current: HashSet<String> = registry.clone().get().await.keys().cloned().collect();
        let wanted: HashSet<&str> = desired.iter().map(|vm| vm.key.as_str()).collect();

        let mut outcome = ReconcileOutcome::default();
        for vm in desired {
            if current.contains(vm.key.as_str()) {
                continue;
            }
            let command = self.materialize(&vm.key, &vm.spec).await?;
            self.tx
                .send(Command::Create(command))
                .await
                .map_err(|_| ReconcileError::ChannelClosed)?;
            outcome.created.push(vm.key.clone());
        }

        let mut extra: Vec<&String> = current
            .iter()
            .filter(|id| !wanted.contains(id.as_str()))
            .collect();
        extra.sort();
        for id in extra {
            self.tx
                .send(Command::Delete(id.clone()))
                .await
                .map_err(|_| ReconcileError::ChannelClosed)?;
            outcome.deleted.push(id.clone());
        }
        Ok(outcome)
    }

    /// Builds a key-addressed creation command: decodes the spec, asks the
    /// factory to prepare the VM, then re-keys the factory-assigned id
    /// (factories mint their own ids, e.g. uuids) to the desired key.
    async fn materialize(
        &self,
        key: &str,
        bytes: &[u8],
    ) -> Result<CreateCommand<F>, ReconcileError> {
        let spec_error = |err: capnp::Error| ReconcileError::Spec(format!("{key}: {err}"));
        let message = capnp::serialize::read_message(&mut &bytes[..], ReaderOptions::default())
            .map_err(spec_error)?;
        let root = message
            .get_root::<commands::common_capnp::vm_spec::Reader<F::BackendConfig>>()
            .map_err(spec_error)?;
        let spec = F::CreateVmSpec::try_from(root).map_err(spec_error)?;
        let command = self
            .factory
            .create_vm(spec)
            .await
            .map_err(ReconcileError::Factory)?;
        let (handle, _) = command.into_parts();
        Ok(CreateCommand::new(handle, key.to_owned()))
    }
}

/// Capped exponential backoff: `base * 2^failures` up to 30 seconds. Pure and
/// unit-tested; the loops only sleep on it.
pub(crate) fn backoff_delay(failures: u32, base: Duration) -> Duration {
    base.saturating_mul(1_u32 << failures.min(5))
        .min(MAX_BACKOFF)
}

/// Signal returned when the connection is lost (caller reconnects).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportLost;

/// Polls until the connection breaks: fetches, reconciles new generations,
/// and backs off on rejections or reconcile failures. Silence on unchanged
/// generations (no-op, still sleeps the poll interval). Applied generations
/// land in the shared cell for the heartbeat task.
pub async fn poll_loop<C, F>(
    client: &C,
    reconciler: &Reconciler<F>,
    registry: &Registry<F, Reader>,
    worker_id: &str,
    poll_interval: Duration,
    applied: &AppliedGeneration,
) -> TransportLost
where
    C: PullClient,
    F: Factory,
{
    let mut failures: u32 = 0;
    loop {
        match client.fetch(worker_id, applied.get().unwrap_or(0)).await {
            Ok(assignment) => {
                failures = 0;
                if applied.get() != Some(assignment.generation) {
                    match reconciler.reconcile(&assignment.desired, registry).await {
                        Ok(outcome) => {
                            applied.set(assignment.generation);
                            tracing::info!(
                                worker = worker_id,
                                generation = assignment.generation,
                                created = outcome.created.len(),
                                deleted = outcome.deleted.len(),
                                "Reconciled assignment"
                            );
                        }
                        Err(err) => {
                            failures += 1;
                            tracing::warn!(worker = worker_id, %err, "Reconcile failed");
                            sleep(backoff_delay(failures, poll_interval)).await;
                            continue;
                        }
                    }
                }
                sleep(poll_interval).await;
            }
            Err(PullError::Rejected(message)) => {
                failures += 1;
                tracing::warn!(worker = worker_id, %message, "Assignment rejected");
                sleep(backoff_delay(failures, poll_interval)).await;
            }
            Err(PullError::Transport(reason)) => {
                tracing::warn!(worker = worker_id, %reason, "Master connection lost");
                return TransportLost;
            }
        }
    }
}

/// Dials the control plane and polls forever, redialing with backoff after
/// dial failures and pausing one interval after transport breaks (the next
/// dial then fails fast with backoff if the outage persists).
pub async fn run_poll_loop<F>(
    config: PollConfig,
    reconciler: Reconciler<F>,
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
                poll_loop(
                    &client,
                    &reconciler,
                    &registry,
                    &config.worker_id,
                    config.poll_interval,
                    &applied,
                )
                .await;
                sleep(config.poll_interval).await;
            }
            Err(err) => {
                failures += 1;
                tracing::warn!(
                    master = %config.master_addr,
                    %err,
                    failures,
                    "Master unreachable"
                );
                sleep(backoff_delay(failures, config.poll_interval)).await;
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
    use tokio::sync::mpsc::{self, Receiver};
    use tokio_util::compat::TokioAsyncReadCompatExt;

    use super::{
        AppliedGeneration, DesiredVm, PullClient, PullError, PulledAssignment, ReconcileError,
        Reconciler, RpcPullClient, TransportLost, backoff_delay, poll_loop,
    };
    use crate::{
        test_support::{FailFactory, FakeFactory, FakeHandle, TempDb},
        vmm::{Command, Factory, Reader, Registry, Writer},
    };

    fn desired_vm(key: &str) -> DesiredVm {
        DesiredVm {
            key: key.to_string(),
            spec: fake_spec_bytes(),
        }
    }

    fn fake_spec_bytes() -> Vec<u8> {
        // Empty default message: the fake backend ignores content entirely.
        // init_root allocates the first segment (a zero-segment message
        // underflows inside write_message).
        let mut message = capnp::message::Builder::new_default();
        let _root: commands::common_capnp::vm_spec::Builder<'_, capnp::any_pointer::Owned> =
            message.init_root();
        let mut bytes = Vec::new();
        capnp::serialize::write_message(&mut bytes, &message).expect("serialize test spec");
        assert!(!bytes.is_empty(), "even empty specs encode segments");
        bytes
    }

    fn assignment(generation: u64, keys: &[&str]) -> PulledAssignment {
        PulledAssignment {
            generation,
            desired: keys.iter().map(|key| desired_vm(key)).collect(),
        }
    }

    async fn test_registry<F>(names: &[&str]) -> (Registry<F, Reader>, Registry<F, Writer>, TempDb)
    where
        F: Factory<VmHandle = FakeHandle>,
    {
        let (db, guard) = TempDb::create("pull").await;
        let registry = Registry::new(db);
        let (reader, mut writer) = registry.split();
        for name in names {
            writer
                .insert((*name).to_string(), FakeHandle::healthy())
                .await;
        }
        (reader, writer, guard)
    }

    async fn recv_command(rx: &mut Receiver<Command<FakeFactory>>) -> Command<FakeFactory> {
        tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("command arrives in time")
            .expect("channel stays open")
    }

    #[derive(Debug, Default)]
    struct MockClient {
        calls: Mutex<Vec<(String, u64)>>,
        script: Mutex<VecDeque<Result<PulledAssignment, PullError>>>,
        last: Mutex<Option<Result<PulledAssignment, PullError>>>,
    }

    impl MockClient {
        fn scripted(responses: Vec<Result<PulledAssignment, PullError>>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                script: Mutex::new(responses.into()),
                last: Mutex::new(None),
            }
        }

        fn calls(&self) -> Vec<(String, u64)> {
            self.calls.lock().expect("calls").clone()
        }
    }

    impl PullClient for MockClient {
        async fn fetch(
            &self,
            worker_id: &str,
            last_seen_generation: u64,
        ) -> Result<PulledAssignment, PullError> {
            self.calls
                .lock()
                .expect("calls")
                .push((worker_id.to_owned(), last_seen_generation));
            let mut script = self.script.lock().expect("script");
            let mut last = self.last.lock().expect("last");
            match script.pop_front() {
                Some(response) => {
                    *last = Some(response.clone());
                    response
                }
                None => last.clone().expect("script exhausted"),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reconciler_creates_missing_and_deletes_extra() {
        let (reader, _writer, _db) = test_registry::<FakeFactory>(&["keep", "stale"]).await;
        let (tx, mut rx) = mpsc::channel(16);
        let reconciler = Reconciler::new(FakeFactory, tx);

        let outcome = reconciler
            .reconcile(&[desired_vm("keep"), desired_vm("new")], &reader)
            .await
            .expect("reconcile");
        assert_eq!(outcome.created, vec!["new".to_string()]);
        assert_eq!(outcome.deleted, vec!["stale".to_string()]);

        match recv_command(&mut rx).await {
            Command::Create(cmd) => assert_eq!(cmd.id(), "new"),
            Command::Delete(id) => panic!("expected Create first, got Delete({id})"),
        }
        match recv_command(&mut rx).await {
            Command::Delete(id) => assert_eq!(id, "stale"),
            Command::Create(cmd) => panic!("expected Delete second, got Create({})", cmd.id()),
        }
        assert!(rx.try_recv().is_err(), "no further commands");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reconciler_applies_then_noops() {
        let (reader, mut writer, _db) = test_registry::<FakeFactory>(&[]).await;
        let (tx, mut rx) = mpsc::channel(16);
        let reconciler = Reconciler::new(FakeFactory, tx);
        let desired = vec![desired_vm("vm-a")];

        let outcome = reconciler
            .reconcile(&desired, &reader)
            .await
            .expect("first reconcile");
        assert_eq!(outcome.created, vec!["vm-a".to_string()]);
        let _ = recv_command(&mut rx).await;

        // Simulate the supervisor applying the command.
        writer
            .insert("vm-a".to_string(), FakeHandle::healthy())
            .await;

        let outcome = reconciler
            .reconcile(&desired, &reader)
            .await
            .expect("second reconcile");
        assert!(outcome.created.is_empty() && outcome.deleted.is_empty());
        assert!(rx.try_recv().is_err(), "converged state stays silent");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reconciler_create_failure_reports_without_sending() {
        let (reader, _writer, _db) = test_registry::<FailFactory>(&[]).await;
        let (tx, mut rx) = mpsc::channel(16);
        let reconciler = Reconciler::new(FailFactory, tx);

        match reconciler.reconcile(&[desired_vm("vm-a")], &reader).await {
            Err(ReconcileError::Factory(_)) => {}
            other => panic!("expected factory failure, got {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "failed creates send nothing");
    }

    #[test]
    fn backoff_delay_grows_and_caps() {
        let base = Duration::from_secs(2);
        assert_eq!(backoff_delay(1, base), Duration::from_secs(4));
        assert_eq!(backoff_delay(2, base), Duration::from_secs(8));
        assert_eq!(backoff_delay(5, base), Duration::from_secs(30));
        assert_eq!(backoff_delay(99, base), Duration::from_secs(30));
    }

    /// Mini-supervisor for loop tests: applies channel commands to the writer
    /// registry and logs ids in order (single-threaded runtime keeps this
    /// deterministic).
    async fn apply_loop(
        mut rx: mpsc::Receiver<Command<FakeFactory>>,
        mut writer: Registry<FakeFactory, Writer>,
        log: Arc<Mutex<Vec<String>>>,
    ) {
        while let Some(cmd) = rx.recv().await {
            match cmd {
                Command::Create(cmd) => {
                    let (handle, id) = cmd.into_parts();
                    log.lock().expect("log").push(format!("C:{id}"));
                    writer.insert(id, handle).await;
                }
                Command::Delete(id) => {
                    log.lock().expect("log").push(format!("D:{id}"));
                    writer.remove(&id).await;
                }
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn poll_loop_creates_noops_retries_then_deletes() {
        let (reader, writer, _db) = test_registry::<FakeFactory>(&[]).await;
        let (tx, rx) = mpsc::channel(16);
        let reconciler = Reconciler::new(FakeFactory, tx);
        let log = Arc::new(Mutex::new(Vec::new()));
        let applier = tokio::spawn(apply_loop(rx, writer, log.clone()));
        let client = MockClient::scripted(vec![
            Ok(assignment(1, &["vm-a"])),
            Ok(assignment(1, &["vm-a"])),
            Err(PullError::Rejected("node busy".to_string())),
            Ok(assignment(2, &[])),
        ]);

        let outcome = tokio::time::timeout(
            Duration::from_millis(500),
            poll_loop(
                &client,
                &reconciler,
                &reader,
                "w1",
                Duration::from_millis(5),
                &AppliedGeneration::new(),
            ),
        )
        .await;
        assert!(outcome.is_err(), "loop must not return on its own");

        // Clean applier shutdown: dropping the reconciler closes the channel.
        drop(reconciler);
        applier.await.expect("applier exits cleanly");

        // Create applied once, delete applied once, in order. Same-generation
        // polls skip reconciliation entirely, so creates never repeat; a
        // duplicate delete is possible only if the applier stalls past a poll
        // interval, hence the prefix form.
        let logged: Vec<String> = log.lock().expect("log").clone();
        assert!(logged.len() >= 2, "unexpected log: {logged:?}");
        assert_eq!(&logged[..2], &["C:vm-a".to_string(), "D:vm-a".to_string()]);
        assert_eq!(
            logged.iter().filter(|entry| entry.starts_with('C')).count(),
            1,
            "creates must not repeat: {logged:?}"
        );
        assert!(
            logged
                .iter()
                .all(|entry| entry == "C:vm-a" || entry == "D:vm-a"),
            "unexpected log: {logged:?}"
        );
        // End state converged back to empty.
        assert!(reader.clone().get().await.keys().next().is_none());

        let calls = client.calls();
        assert!(calls.len() >= 4, "unexpected calls: {calls:?}");
        let expected = vec![
            ("w1".to_string(), 0),
            ("w1".to_string(), 1),
            ("w1".to_string(), 1),
            ("w1".to_string(), 1),
        ];
        assert_eq!(&calls[..4], &expected);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn poll_loop_returns_on_transport_loss() {
        let (reader, _writer, _db) = test_registry::<FakeFactory>(&[]).await;
        let (tx, _rx) = mpsc::channel(16);
        let reconciler = Reconciler::new(FakeFactory, tx);
        let client = MockClient::scripted(vec![Err(PullError::Transport("boom".to_string()))]);

        let result = poll_loop(
            &client,
            &reconciler,
            &reader,
            "w1",
            Duration::from_millis(5),
            &AppliedGeneration::new(),
        )
        .await;
        assert_eq!(result, TransportLost);
        assert_eq!(client.calls(), vec![("w1".to_string(), 0)]);
    }

    #[derive(Clone)]
    enum StubMode {
        Assign { generation: u64, ids: Vec<String> },
        Reject(String),
    }

    #[derive(Clone)]
    struct StubMaster {
        mode: Arc<Mutex<StubMode>>,
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
            params: commands::master_capnp::master::GetAssignmentParams<capnp::any_pointer::Owned>,
            mut results: commands::master_capnp::master::GetAssignmentResults<
                capnp::any_pointer::Owned,
            >,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            let mode = self.mode.lock().expect("stub mode").clone();
            capnp::capability::Promise::from_future(async move {
                let params = params.get()?;
                let _ = (params.get_worker_id()?, params.get_last_seen_generation());
                match mode {
                    StubMode::Assign { generation, ids } => {
                        let mut ok = results.get().get_result()?.init_ok();
                        ok.set_generation(generation);
                        let count = u32::try_from(ids.len()).unwrap_or(u32::MAX);
                        let mut specs = ok.init_desired_vms(count);
                        for (index, id) in ids.iter().enumerate() {
                            let index = u32::try_from(index).unwrap_or(u32::MAX);
                            let mut slot = specs.reborrow().get(index);
                            slot.set_id(id.as_str());
                            slot.set_target_worker("w1");
                        }
                        Ok(())
                    }
                    StubMode::Reject(message) => {
                        results.get().get_result()?.set_err(message.as_str())?;
                        Ok(())
                    }
                }
            })
        }

        fn push_data(
            &mut self,
            _: commands::master_capnp::master::PushDataParams<capnp::any_pointer::Owned>,
            _: commands::master_capnp::master::PushDataResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            capnp::capability::Promise::err(capnp::Error::failed("stub".to_string()))
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
    async fn rpc_pull_client_roundtrip_with_wire_ids() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let mode = Arc::new(Mutex::new(StubMode::Assign {
                    generation: 7,
                    ids: vec!["vm-a".to_string(), String::new(), "vm-b".to_string()],
                }));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("bind");
                let addr = listener.local_addr().expect("addr");
                let stub = StubMaster { mode: mode.clone() };
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

                let client = RpcPullClient::<capnp::any_pointer::Owned>::connect(addr)
                    .await
                    .expect("connect");
                let assignment = client.fetch("w1", 0).await.expect("fetch");
                assert_eq!(assignment.generation, 7);
                // Wire ids become keys; the blank one is skipped, not poisoned.
                assert_eq!(
                    assignment
                        .desired
                        .iter()
                        .map(|vm| vm.key.as_str())
                        .collect::<Vec<_>>(),
                    vec!["vm-a", "vm-b"]
                );
                assert!(assignment.desired.iter().all(|vm| !vm.spec.is_empty()));

                // Keys reconcile into creates end to end.
                let (reader, _writer, _db) = test_registry::<FakeFactory>(&[]).await;
                let (tx, _rx) = mpsc::channel(16);
                let reconciler = Reconciler::new(FakeFactory, tx);
                let outcome = reconciler
                    .reconcile(&assignment.desired, &reader)
                    .await
                    .expect("reconcile");
                assert_eq!(
                    outcome.created,
                    vec!["vm-a".to_string(), "vm-b".to_string()]
                );

                *mode.lock().expect("stub mode") = StubMode::Reject("node busy".to_string());
                match client.fetch("w1", 7).await {
                    Err(PullError::Rejected(message)) => {
                        assert!(message.contains("node busy"), "unexpected: {message}");
                    }
                    other => panic!("expected rejection, got {other:?}"),
                }
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rpc_pull_client_connect_refused() {
        match RpcPullClient::<capnp::any_pointer::Owned>::connect(
            "127.0.0.1:1".parse().expect("addr"),
        )
        .await
        {
            Err(PullError::Transport(_)) => {}
            Err(other) => panic!("expected transport failure, got rejection: {other:?}"),
            Ok(_) => panic!("expected connection failure"),
        }
    }
}
