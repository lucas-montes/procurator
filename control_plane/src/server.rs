//! Central point of communication. Talks to workers and receives requests from the cli.
//!
//! Each `Master` handler is thin translation: extract owned values from the
//! Cap'n Proto readers, forward a domain event over the node channel, and map
//! the typed reply back onto the wire. Content rejections (bad ids, deferred
//! payloads) answer through the `Result` union; only broken framing or a lost
//! node channel becomes a `Promise` failure.
//!
//! Narrowed T04 semantics (schema frozen): `publishState` maps
//! commit/generation/intent and requires empty `vmSpecs` — specs carry no vm
//! identity or target worker until the spec follow-up, and they fail loudly
//! instead of being dropped silently. `getAssignment` echoes the generation
//! with an empty spec list. `pushData` merges keys plus liveness; per-VM and
//! resource metrics are debug-logged and gain stored fields in the follow-up.
//! `ClusterStatus.vms` stays empty for the same reason. `getWorker` resolves
//! through [`WorkerDirectory`], unavailable in pull-mode v1.

use std::{net::SocketAddr, sync::Arc};

use capnp::message::ReaderOptions;
use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use futures::AsyncReadExt;
use tracing::{debug, info, instrument};

use crate::{
    directory::{NoWorkerDirectory, WorkerDirectory},
    dto::{NodeEvent, NodeMessenger, NodeReply, ObservedReport, RequestError},
    registry::WorkerId,
    store::{DesiredEntry, Generation, VmKey},
};

#[derive(Clone)]
pub struct Server<T> {
    messenger: NodeMessenger,
    directory: Arc<dyn WorkerDirectory>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: capnp::traits::Owned + 'static> Server<T> {
    pub fn new(messenger: impl Into<NodeMessenger>) -> Self {
        Server {
            messenger: messenger.into(),
            directory: Arc::new(NoWorkerDirectory),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Serves `getWorker` lookups from `directory` (e.g. a push transport's
    /// live clients); pull-mode default is [`NoWorkerDirectory`].
    // Swappable seam: exercised by tests; a push transport injects clients.
    #[allow(dead_code)]
    pub fn with_directory(mut self, directory: Arc<dyn WorkerDirectory>) -> Self {
        self.directory = directory;
        self
    }

    #[instrument(skip(self))]
    pub async fn serve(self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        info!(addr = %addr, "Starting server");
        let listener = tokio::net::TcpListener::bind(&addr).await?;

        let client: commands::master_capnp::master::Client<T> = capnp_rpc::new_client(self);

        loop {
            let (stream, peer_addr) = listener.accept().await?;
            debug!(peer_addr = %peer_addr, "New connection");
            stream.set_nodelay(true)?;
            let (reader, writer) =
                tokio_util::compat::TokioAsyncReadCompatExt::compat(stream).split();
            let network = twoparty::VatNetwork::new(
                futures::io::BufReader::new(reader),
                futures::io::BufWriter::new(writer),
                rpc_twoparty_capnp::Side::Server,
                ReaderOptions::default(),
            );

            // TODO: Determine which client to provide based on connection context
            // For now, defaulting to master_control for CLI connections
            let rpc_system = RpcSystem::new(Box::new(network), Some(client.clone().client));

            tokio::task::spawn_local(rpc_system);
        }
    }
}

/// Maps a lost node channel to a Cap'n Proto transport failure.
fn transport_error(err: RequestError) -> capnp::Error {
    capnp::Error::failed(format!("{err}"))
}

/// Reads a text field, mapping non-UTF8 to a Cap'n Proto failure.
fn read_text(reader: capnp::text::Reader<'_>) -> Result<String, capnp::Error> {
    reader
        .to_str()
        .map(str::to_owned)
        .map_err(|err| capnp::Error::failed(format!("invalid utf-8 text field: {err}")))
}

/// Serializes one borrowed spec into owned bytes for Node retention.
fn serialize_spec<T: capnp::traits::Owned>(
    spec: commands::common_capnp::vm_spec::Reader<'_, T>,
) -> Result<Vec<u8>, capnp::Error> {
    let mut message = capnp::message::Builder::new_default();
    message.set_root(spec)?;
    let mut bytes = Vec::new();
    capnp::serialize::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

impl<T: capnp::traits::Owned + 'static> commands::master_capnp::master::Server<T> for Server<T> {
    fn publish_state(
        &mut self,
        params: commands::master_capnp::master::PublishStateParams<T>,
        mut results: commands::master_capnp::master::PublishStateResults<T>,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        let messenger = self.messenger.clone();
        ::capnp::capability::Promise::from_future(async move {
            let params = params.get()?;
            let commit = read_text(params.get_commit()?)?;
            let generation = Generation::new(params.get_generation());
            let intent_hash = read_text(params.get_intent_hash()?)?;
            let wire_specs = params.get_vm_specs()?;

            info!(
                generation = generation.value(),
                ?commit,
                ?intent_hash,
                specs = wire_specs.len(),
                "Publish request"
            );

            // Specs carry explicit identity (id) and placement (targetWorker);
            // blank ids and unparseable targets fail loudly through the
            // Result union, and undeclared targets fail in Node. Payloads are
            // retained opaquely for getAssignment rebuilds (plan T09).
            let mut entries = Vec::new();
            for spec in wire_specs {
                let raw_id = read_text(spec.get_id()?)?;
                let key = match VmKey::new(raw_id) {
                    Ok(key) => key,
                    Err(err) => {
                        results
                            .get()
                            .get_result()?
                            .set_err(format!("{err}").as_str())?;
                        return Ok(());
                    }
                };
                let raw_target = read_text(spec.get_target_worker()?)?;
                let target = match WorkerId::new(raw_target) {
                    Ok(target) => target,
                    Err(err) => {
                        results
                            .get()
                            .get_result()?
                            .set_err(format!("{err}").as_str())?;
                        return Ok(());
                    }
                };
                let bytes = serialize_spec(spec)
                    .map_err(|err| capnp::Error::failed(format!("serialize spec {key}: {err}")))?;
                entries.push(DesiredEntry::new(key, target).with_spec(bytes));
            }

            let reply = messenger
                .request(NodeEvent::PublishState {
                    commit,
                    generation,
                    intent_hash,
                    entries,
                })
                .await
                .map_err(transport_error)?;
            match reply {
                Err(node_err) => {
                    results
                        .get()
                        .get_result()?
                        .set_err(format!("{node_err}").as_str())?;
                    return Ok(());
                }
                Ok(NodeReply::Published(active)) => {
                    debug!(generation = active.value(), "Publish stored");
                    let _ = results.get().get_result()?.init_ok();
                }
                Ok(unexpected) => {
                    return Err(capnp::Error::failed(format!(
                        "node returned unexpected reply for publishState: {unexpected:?}"
                    )));
                }
            }
            Ok(())
        })
    }

    fn get_assignment(
        &mut self,
        params: commands::master_capnp::master::GetAssignmentParams<T>,
        mut results: commands::master_capnp::master::GetAssignmentResults<T>,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        let messenger = self.messenger.clone();
        ::capnp::capability::Promise::from_future(async move {
            let params = params.get()?;
            let raw_id = read_text(params.get_worker_id()?)?;
            let last_seen_generation = params.get_last_seen_generation();

            debug!(worker = %raw_id, last_seen_generation, "Getting assignment");

            let worker_id = match WorkerId::new(raw_id) {
                Ok(id) => id,
                Err(err) => {
                    results
                        .get()
                        .get_result()?
                        .set_err(format!("{err}").as_str())?;
                    return Ok(());
                }
            };

            let reply = messenger
                .request(NodeEvent::GetAssignment {
                    worker_id: worker_id.clone(),
                    last_seen_generation,
                })
                .await
                .map_err(transport_error)?;
            let assignment = match reply {
                Err(node_err) => {
                    results
                        .get()
                        .get_result()?
                        .set_err(format!("{node_err}").as_str())?;
                    return Ok(());
                }
                Ok(NodeReply::Assignment(assignment)) => assignment,
                Ok(unexpected) => {
                    return Err(capnp::Error::failed(format!(
                        "node returned unexpected reply for getAssignment: {unexpected:?}"
                    )));
                }
            };

            let mut ok = results.get().get_result()?.init_ok();
            ok.set_generation(assignment.generation().map_or(0, Generation::value));
            // Rebuild each desiredVm from its retained payload: validated key
            // and requesting worker for identity (they equal the stored
            // texts by construction), opaque backend bytes for the spec.
            // Note the response list is branded any_pointer (generated
            // code), so the backend crosses via set_as.
            let vms = assignment.vms();
            let mut desired = ok.init_desired_vms(u32::try_from(vms.len()).unwrap_or(u32::MAX));
            for (index, vm) in vms.iter().enumerate() {
                let message =
                    capnp::serialize::read_message(&mut &vm.spec()[..], ReaderOptions::default())
                        .map_err(|err| {
                        capnp::Error::failed(format!("stored spec for {}: {err}", vm.key()))
                    })?;
                let stored = message
                    .get_root::<commands::common_capnp::vm_spec::Reader<T>>()
                    .map_err(|err| {
                        capnp::Error::failed(format!("stored spec for {}: {err}", vm.key()))
                    })?;
                let mut slot = desired
                    .reborrow()
                    .get(u32::try_from(index).expect("assignment index fits in u32"));
                slot.set_id(vm.key().as_str());
                slot.set_target_worker(worker_id.as_str());
                slot.init_spec()
                    .set_as::<T>(stored.get_spec()?)
                    .map_err(|err| {
                        capnp::Error::failed(format!("stored spec for {}: {err}", vm.key()))
                    })?;
            }
            Ok(())
        })
    }

    fn push_data(
        &mut self,
        params: commands::master_capnp::master::PushDataParams<T>,
        mut results: commands::master_capnp::master::PushDataResults<T>,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        let messenger = self.messenger.clone();
        ::capnp::capability::Promise::from_future(async move {
            let params = params.get()?;
            let raw_id = read_text(params.get_worker_id()?)?;
            let observed_generation = params.get_observed_generation();

            let worker_id = match WorkerId::new(raw_id) {
                Ok(id) => id,
                Err(err) => {
                    results
                        .get()
                        .get_result()?
                        .set_err(format!("{err}").as_str())?;
                    return Ok(());
                }
            };

            let wire_vms = params.get_running_vms()?;
            let mut running = Vec::new();
            for vm in wire_vms {
                let id = read_text(vm.get_id()?)?;
                match VmKey::new(id) {
                    Ok(key) => running.push(key),
                    Err(err) => {
                        results
                            .get()
                            .get_result()?
                            .set_err(format!("{err}").as_str())?;
                        return Ok(());
                    }
                }
            }
            let metrics = params.get_metrics()?;
            debug!(
                worker = %worker_id,
                observed_generation,
                running = running.len(),
                cpu = metrics.get_available_cpu(),
                memory = metrics.get_available_memory(),
                disk = metrics.get_disk_usage(),
                uptime = metrics.get_uptime(),
                "Worker pushing data"
            );

            // Narrowed T04: Node keeps keys plus liveness only; `healthy`
            // here means "sent a heartbeat". Metrics gain stored fields in
            // the spec follow-up.
            let report = ObservedReport::new(worker_id, observed_generation, running, true);
            let reply = messenger
                .request(NodeEvent::PushData { report })
                .await
                .map_err(transport_error)?;
            match reply {
                Err(node_err) => {
                    results
                        .get()
                        .get_result()?
                        .set_err(format!("{node_err}").as_str())?;
                }
                Ok(NodeReply::Observed) => {
                    let _ = results.get().get_result()?.init_ok();
                }
                Ok(unexpected) => {
                    return Err(capnp::Error::failed(format!(
                        "node returned unexpected reply for pushData: {unexpected:?}"
                    )));
                }
            }
            Ok(())
        })
    }

    fn get_cluster_status(
        &mut self,
        params: commands::master_capnp::master::GetClusterStatusParams<T>,
        mut results: commands::master_capnp::master::GetClusterStatusResults<T>,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        let messenger = self.messenger.clone();
        ::capnp::capability::Promise::from_future(async move {
            params.get()?;
            debug!("Getting cluster status");

            let reply = messenger
                .request(NodeEvent::GetClusterStatus)
                .await
                .map_err(transport_error)?;
            let snapshot = match reply {
                Err(node_err) => {
                    return Err(capnp::Error::failed(format!("{node_err}")));
                }
                Ok(NodeReply::Cluster(snapshot)) => snapshot,
                Ok(unexpected) => {
                    return Err(capnp::Error::failed(format!(
                        "node returned unexpected reply for getClusterStatus: {unexpected:?}"
                    )));
                }
            };

            let mut status = results.get().init_status();
            status.set_active_generation(snapshot.generation().map_or(0, Generation::value));
            status.set_active_commit(snapshot.commit());
            status.set_convergence_percent(snapshot.convergence_percent());
            // Narrowed T04: per-VM status rows deferred with the spec
            // follow-up (Node tracks keys, not hashes or VM state).
            let _ = status.reborrow().init_vms(0);
            let views = snapshot.workers();
            let mut workers = status.init_workers(u32::try_from(views.len()).unwrap_or(u32::MAX));
            for (index, view) in views.iter().enumerate() {
                let mut worker = workers
                    .reborrow()
                    .get(u32::try_from(index).expect("worker index fits in u32"));
                worker.set_id(view.worker_id().as_str());
                let reported = view.reported();
                worker.set_healthy(reported.is_some_and(ObservedReport::is_healthy));
                worker.set_generation(reported.map_or(0, ObservedReport::observed_generation));
                worker.set_running_vms(reported.map_or(0, |report| {
                    u32::try_from(report.running().len()).unwrap_or(u32::MAX)
                }));
                // Narrowed T04: resource and metric detail deferred.
                let mut resources = worker.reborrow().init_available_resources();
                resources.set_cpu(0.0);
                resources.set_memory_bytes(0);
                let mut metrics = worker.reborrow().init_metrics();
                metrics.set_available_cpu(0.0);
                metrics.set_available_memory(0);
                metrics.set_disk_usage(0);
                metrics.set_uptime(0);
            }
            Ok(())
        })
    }

    fn get_worker(
        &mut self,
        params: commands::master_capnp::master::GetWorkerParams<T>,
        mut results: commands::master_capnp::master::GetWorkerResults<T>,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        let messenger = self.messenger.clone();
        let directory = self.directory.clone();
        ::capnp::capability::Promise::from_future(async move {
            let params = params.get()?;
            let raw_id = read_text(params.get_worker_id()?)?;
            debug!(worker = %raw_id, "Getting worker capability");

            let worker_id =
                WorkerId::new(raw_id).map_err(|err| capnp::Error::failed(format!("{err}")))?;
            let reply = messenger
                .request(NodeEvent::GetWorker {
                    worker_id: worker_id.clone(),
                })
                .await
                .map_err(transport_error)?;
            let view = match reply {
                Err(node_err) => {
                    return Err(capnp::Error::failed(format!("{node_err}")));
                }
                Ok(NodeReply::Worker(view)) => view,
                Ok(unexpected) => {
                    return Err(capnp::Error::failed(format!(
                        "node returned unexpected reply for getWorker: {unexpected:?}"
                    )));
                }
            };
            if view.is_none() {
                return Err(capnp::Error::failed(format!("unknown worker: {worker_id}")));
            }
            let client = directory
                .lookup(&worker_id)
                .map_err(|err| capnp::Error::failed(format!("{err}")))?;
            results.get().set_worker(client);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use capnp::message::ReaderOptions;
    use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
    use futures::AsyncReadExt;
    use tokio::sync::mpsc::channel;
    use tokio_util::compat::TokioAsyncReadCompatExt;

    use super::Server;
    use crate::{
        directory::{DirectoryError, NoWorkerDirectory, WorkerDirectory},
        dto::NodeMessenger,
        node::Node,
        registry::{DeclaredWorker, WorkerId},
    };

    type MasterClient = commands::master_capnp::master::Client<capnp::any_pointer::Owned>;
    type ChMasterClient =
        commands::master_capnp::master::Client<commands::ch_capnp::vm_config::Owned>;
    type WorkerClient = commands::worker_capnp::worker::Client<capnp::any_pointer::Owned>;

    fn declared(name: &str) -> DeclaredWorker {
        DeclaredWorker::new(WorkerId::new(name).expect("test worker id must be valid"))
    }

    /// Binds an ephemeral loopback port, serves one `Master` on it, and
    /// returns the address. Runs inside the caller's `LocalSet`.
    async fn spawn_master_server<T>(
        declared_workers: Vec<DeclaredWorker>,
        directory: Arc<dyn WorkerDirectory>,
    ) -> std::net::SocketAddr
    where
        T: capnp::traits::Owned + 'static,
    {
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("probe bind");
        let addr = probe.local_addr().expect("probe addr");
        drop(probe);

        let (tx, rx) = channel(16);
        let node = Node::new(rx, Vec::new(), declared_workers);
        let _node_task = tokio::spawn(node.run());
        let server = Server::<T>::new(NodeMessenger::from(tx)).with_directory(directory);
        let _serve_task = tokio::task::spawn_local(server.serve(addr));
        // Yield once so the listener binds before the client dials.
        tokio::task::yield_now().await;
        addr
    }

    async fn connect_master<T>(
        addr: std::net::SocketAddr,
    ) -> commands::master_capnp::master::Client<T>
    where
        T: capnp::traits::Owned,
    {
        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        stream.set_nodelay(true).expect("nodelay");
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
        client
    }

    fn rpc_text(err: impl std::fmt::Display) -> String {
        format!("rpc: {err}")
    }

    /// Decodes a `Result(Empty, Text)` union into `Ok(())` / `Err(text)`.
    fn decode_empty(
        result: commands::common_capnp::result::Reader<
            commands::common_capnp::empty::Owned,
            capnp::text::Owned,
        >,
    ) -> Result<(), String> {
        match result
            .which()
            .map_err(|_| "schema discriminant".to_string())?
        {
            commands::common_capnp::result::Which::Ok(_) => Ok(()),
            commands::common_capnp::result::Which::Err(text) => Err(text
                .map_err(rpc_text)?
                .to_str()
                .map_err(rpc_text)?
                .to_owned()),
        }
    }

    async fn publish_ch(
        client: &ChMasterClient,
        commit: &str,
        generation: u64,
        specs: &[(&str, &str, &str)],
    ) -> Result<(), String> {
        let mut request = client.publish_state_request();
        {
            let mut params = request.get();
            params.set_commit(commit);
            params.set_generation(generation);
            params.set_intent_hash("intent");
            let count = u32::try_from(specs.len()).unwrap_or(u32::MAX);
            let mut wire = params.reborrow().init_vm_specs(count);
            for (index, (id, target, kernel)) in specs.iter().enumerate() {
                let index = u32::try_from(index).unwrap_or(u32::MAX);
                let mut slot = wire.reborrow().get(index);
                slot.set_id(id);
                slot.set_target_worker(target);
                let mut payload = slot.reborrow().init_spec();
                {
                    let mut payload_config = payload.reborrow().init_payload();
                    payload_config.set_kernel(kernel);
                    payload_config.set_cmdline("console=ttyS0");
                    payload_config.set_initramfs("initrd");
                }
                {
                    let mut cpus = payload.reborrow().init_cpus();
                    cpus.set_boot_vcpus(2);
                    cpus.set_max_vcpus(2);
                }
                {
                    let mut memory = payload.reborrow().init_memory();
                    memory.set_size(1024);
                }
            }
        }
        let response = request.send().promise.await.map_err(rpc_text)?;
        decode_empty(
            response
                .get()
                .map_err(rpc_text)?
                .get_result()
                .map_err(rpc_text)?,
        )
    }

    async fn assignment_ch(
        client: &ChMasterClient,
        worker: &str,
        last_seen: u64,
    ) -> Result<(u64, Vec<(String, String, String)>), String> {
        fn text(reader: capnp::text::Reader<'_>) -> Result<String, String> {
            reader.to_str().map(str::to_owned).map_err(rpc_text)
        }

        let mut request = client.get_assignment_request();
        {
            let mut params = request.get();
            params.set_worker_id(worker);
            params.set_last_seen_generation(last_seen);
        }
        let response = request.send().promise.await.map_err(rpc_text)?;
        match response
            .get()
            .map_err(rpc_text)?
            .get_result()
            .map_err(rpc_text)?
            .which()
            .map_err(|_| "schema discriminant".to_string())?
        {
            commands::common_capnp::result::Which::Ok(assignment) => {
                let assignment = assignment.map_err(rpc_text)?;
                let generation = assignment.get_generation();
                let wire = assignment.get_desired_vms().map_err(rpc_text)?;
                let mut specs = Vec::new();
                for spec in wire {
                    let id = text(spec.get_id().map_err(rpc_text)?)?;
                    let target = text(spec.get_target_worker().map_err(rpc_text)?)?;
                    // Response specs are branded any_pointer (generated
                    // code); convert back to the backend struct for reads.
                    let backend: commands::ch_capnp::vm_config::Reader<'_> = spec
                        .get_spec()
                        .map_err(rpc_text)?
                        .get_as()
                        .map_err(rpc_text)?;
                    let kernel = text(
                        backend
                            .get_payload()
                            .map_err(rpc_text)?
                            .get_kernel()
                            .map_err(rpc_text)?,
                    )?;
                    specs.push((id, target, kernel));
                }
                Ok((generation, specs))
            }
            commands::common_capnp::result::Which::Err(text) => Err(text
                .map_err(rpc_text)?
                .to_str()
                .map_err(rpc_text)?
                .to_owned()),
        }
    }

    async fn push_heartbeat<T>(
        client: &commands::master_capnp::master::Client<T>,
        worker: &str,
        observed_generation: u64,
        vm_ids: &[&str],
    ) -> Result<(), String>
    where
        T: capnp::traits::Owned + Unpin + 'static,
    {
        let mut request = client.push_data_request();
        {
            let mut params = request.get();
            params.set_worker_id(worker);
            params.set_observed_generation(observed_generation);
            {
                let count = u32::try_from(vm_ids.len()).unwrap_or(u32::MAX);
                let mut vms = params.reborrow().init_running_vms(count);
                for (index, id) in vm_ids.iter().enumerate() {
                    let index = u32::try_from(index).unwrap_or(u32::MAX);
                    let mut vm = vms.reborrow().get(index);
                    vm.set_id(id);
                    vm.set_content_hash("hash-a");
                    vm.set_status("running");
                    vm.set_uptime(7);
                    let mut vm_metrics = vm.reborrow().init_metrics();
                    vm_metrics.set_cpu_usage(0.5);
                    vm_metrics.set_memory_usage(8);
                    vm_metrics.set_network_rx_bytes(1);
                    vm_metrics.set_network_tx_bytes(2);
                }
            }
            {
                let mut metrics = params.reborrow().init_metrics();
                metrics.set_available_cpu(0.5);
                metrics.set_available_memory(1024);
                metrics.set_disk_usage(2048);
                metrics.set_uptime(9);
            }
        }
        let response = request.send().promise.await.map_err(rpc_text)?;
        decode_empty(
            response
                .get()
                .map_err(rpc_text)?
                .get_result()
                .map_err(rpc_text)?,
        )
    }

    #[tokio::test(flavor = "current_thread")]
    // Roundtrip covers publish/filter/republish/heartbeat/status in one
    // flow; splitting would re-pay the loopback setup per case.
    #[allow(clippy::too_many_lines)]
    async fn master_roundtrip_serves_node_state() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let addr = spawn_master_server::<commands::ch_capnp::vm_config::Owned>(
                    vec![declared("w1"), declared("w2")],
                    Arc::new(NoWorkerDirectory),
                )
                .await;
                let client: ChMasterClient = connect_master(addr).await;

                // Publish two specs for explicit workers.
                publish_ch(
                    &client,
                    "abc",
                    1,
                    &[("vm-a", "w1", "vmlinux-a"), ("vm-b", "w2", "vmlinux-b")],
                )
                .await
                .expect("publish");

                // Blank ids fail loudly instead of poisoning keys.
                let err = publish_ch(&client, "abc", 2, &[("", "w1", "vmlinux-x")])
                    .await
                    .expect_err("blank id must fail");
                assert!(err.contains("must not be empty"), "unexpected error: {err}");

                // Unknown targets fail loudly instead of dropping VMs.
                let err = publish_ch(&client, "abc", 2, &[("vm-z", "ghost", "vmlinux-x")])
                    .await
                    .expect_err("unknown target must fail");
                assert!(err.contains("undeclared worker"), "unexpected error: {err}");

                // Polls return semantically equal specs per worker.
                let (generation, specs) = assignment_ch(&client, "w1", 0).await.expect("w1 poll");
                assert_eq!(generation, 1);
                assert_eq!(
                    specs,
                    vec![(
                        "vm-a".to_string(),
                        "w1".to_string(),
                        "vmlinux-a".to_string()
                    )]
                );
                let (generation, specs) = assignment_ch(&client, "w2", 0).await.expect("w2 poll");
                assert_eq!(generation, 1);
                assert_eq!(
                    specs,
                    vec![(
                        "vm-b".to_string(),
                        "w2".to_string(),
                        "vmlinux-b".to_string()
                    )]
                );
                let (generation, specs) = assignment_ch(&client, "ghost", 0)
                    .await
                    .expect("ghost poll");
                assert_eq!(generation, 1);
                assert!(specs.is_empty());

                // Republishing replaces wholesale: vm-a moves to w2.
                publish_ch(&client, "def", 2, &[("vm-a", "w2", "vmlinux-a")])
                    .await
                    .expect("republish");
                let (generation, specs) = assignment_ch(&client, "w1", 1).await.expect("w1 repoll");
                assert_eq!(generation, 2);
                assert!(specs.is_empty());
                let (generation, specs) = assignment_ch(&client, "w2", 1).await.expect("w2 repoll");
                assert_eq!(generation, 2);
                assert_eq!(
                    specs,
                    vec![(
                        "vm-a".to_string(),
                        "w2".to_string(),
                        "vmlinux-a".to_string()
                    )]
                );

                // Stale generations are rejected; the active one stands.
                let err = publish_ch(&client, "abc", 1, &[])
                    .await
                    .expect_err("stale publish must fail");
                assert!(err.contains("stale"), "unexpected error: {err}");

                push_heartbeat(&client, "w2", 2, &["vm-a"])
                    .await
                    .expect("push");

                // Blank running-vm ids fail loudly instead of poisoning keys.
                let err = push_heartbeat(&client, "w2", 2, &[""])
                    .await
                    .expect_err("blank vm id must fail");
                assert!(err.contains("vm key"), "unexpected error: {err}");

                let request = client.get_cluster_status_request();
                let response = request
                    .send()
                    .promise
                    .await
                    .map_err(rpc_text)
                    .expect("status rpc");
                let status = response
                    .get()
                    .map_err(rpc_text)
                    .expect("status response")
                    .get_status()
                    .map_err(rpc_text)
                    .expect("status body");
                assert_eq!(status.get_active_generation(), 2);
                assert_eq!(
                    status
                        .get_active_commit()
                        .map_err(rpc_text)
                        .expect("commit")
                        .to_str()
                        .map_err(rpc_text)
                        .expect("commit text"),
                    "def"
                );
                assert_eq!(status.get_convergence_percent(), 50);
                assert_eq!(
                    status
                        .get_workers()
                        .map_err(rpc_text)
                        .expect("workers")
                        .len(),
                    2
                );
                assert_eq!(status.get_vms().map_err(rpc_text).expect("vms").len(), 0);

                // Unknown worker: capability error, not a silent empty.
                let mut request = client.get_worker_request();
                request.get().set_worker_id("ghost");
                match request.send().promise.await {
                    Err(err) => assert!(
                        format!("{err}").contains("unknown worker"),
                        "unexpected error: {err}"
                    ),
                    Ok(_) => panic!("ghost worker must fail"),
                }

                // Declared worker, pull-mode directory: clear unavailability.
                let mut request = client.get_worker_request();
                request.get().set_worker_id("w1");
                match request.send().promise.await {
                    Err(err) => assert!(
                        format!("{err}").contains("pull-only"),
                        "unexpected error: {err}"
                    ),
                    Ok(_) => panic!("w1 must fail with the pull-mode directory"),
                }
            })
            .await;
    }

    /// Stub `Worker` implementation backing directory Ok-path tests.
    struct StubWorkerServer;

    impl commands::worker_capnp::worker::Server<capnp::any_pointer::Owned> for StubWorkerServer {
        fn read(
            &mut self,
            _: commands::worker_capnp::worker::ReadParams<capnp::any_pointer::Owned>,
            _: commands::worker_capnp::worker::ReadResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            capnp::capability::Promise::err(capnp::Error::failed("stub".to_string()))
        }

        fn list_vms(
            &mut self,
            _: commands::worker_capnp::worker::ListVmsParams<capnp::any_pointer::Owned>,
            _: commands::worker_capnp::worker::ListVmsResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            capnp::capability::Promise::err(capnp::Error::failed("stub".to_string()))
        }

        fn create_vm(
            &mut self,
            _: commands::worker_capnp::worker::CreateVmParams<capnp::any_pointer::Owned>,
            _: commands::worker_capnp::worker::CreateVmResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            capnp::capability::Promise::err(capnp::Error::failed("stub".to_string()))
        }

        fn delete_vm(
            &mut self,
            _: commands::worker_capnp::worker::DeleteVmParams<capnp::any_pointer::Owned>,
            _: commands::worker_capnp::worker::DeleteVmResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            capnp::capability::Promise::err(capnp::Error::failed("stub".to_string()))
        }

        fn pause_vm(
            &mut self,
            _: commands::worker_capnp::worker::PauseVmParams<capnp::any_pointer::Owned>,
            _: commands::worker_capnp::worker::PauseVmResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            capnp::capability::Promise::err(capnp::Error::failed("stub".to_string()))
        }

        fn resume_vm(
            &mut self,
            _: commands::worker_capnp::worker::ResumeVmParams<capnp::any_pointer::Owned>,
            _: commands::worker_capnp::worker::ResumeVmResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            capnp::capability::Promise::err(capnp::Error::failed("stub".to_string()))
        }

        fn snapshot_vm(
            &mut self,
            _: commands::worker_capnp::worker::SnapshotVmParams<capnp::any_pointer::Owned>,
            _: commands::worker_capnp::worker::SnapshotVmResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            capnp::capability::Promise::err(capnp::Error::failed("stub".to_string()))
        }

        fn backup_disk(
            &mut self,
            _: commands::worker_capnp::worker::BackupDiskParams<capnp::any_pointer::Owned>,
            _: commands::worker_capnp::worker::BackupDiskResults<capnp::any_pointer::Owned>,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            capnp::capability::Promise::err(capnp::Error::failed("stub".to_string()))
        }
    }

    #[derive(Clone)]
    struct StubDirectory {
        client: WorkerClient,
    }

    impl WorkerDirectory for StubDirectory {
        fn lookup(&self, _worker_id: &WorkerId) -> Result<WorkerClient, DirectoryError> {
            Ok(self.client.clone())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    // Capability clients are thread-local by construction; the `Arc` never
    // leaves this single-threaded test.
    #[allow(clippy::arc_with_non_send_sync)]
    async fn get_worker_returns_injected_capability() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let stub: WorkerClient = capnp_rpc::new_client(StubWorkerServer);
                let addr = spawn_master_server::<capnp::any_pointer::Owned>(
                    vec![declared("w1")],
                    Arc::new(StubDirectory { client: stub }),
                )
                .await;
                let client: MasterClient = connect_master(addr).await;

                let mut request = client.get_worker_request();
                request.get().set_worker_id("w1");
                let response = request.send().promise.await.expect("capability rpc");
                let worker = response
                    .get()
                    .map_err(rpc_text)
                    .expect("capability response")
                    .get_worker()
                    .map_err(rpc_text)
                    .expect("capability body");
                drop(worker);
            })
            .await;
    }

    #[test]
    fn vm_spec_missing_identity_fields_default_to_empty() {
        // Simulates pre-T09 bytes: a VmSpec written without ever touching
        // the additive fields must read them as "" (forward compatibility).
        let mut message = capnp::message::Builder::new_default();
        message.init_root::<commands::common_capnp::vm_spec::Builder<commands::ch_capnp::vm_config::Owned>>();
        let mut bytes = Vec::new();
        capnp::serialize::write_message(&mut bytes, &message).expect("serialize");
        let reader = capnp::serialize::read_message(
            &mut &bytes[..],
            capnp::message::ReaderOptions::default(),
        )
        .expect("deserialize");
        let root = reader
            .get_root::<commands::common_capnp::vm_spec::Reader<commands::ch_capnp::vm_config::Owned>>(
            )
            .expect("root");
        assert_eq!(root.get_id().expect("id").to_str().expect("text"), "");
        assert_eq!(
            root.get_target_worker()
                .expect("target")
                .to_str()
                .expect("text"),
            ""
        );
    }
}
