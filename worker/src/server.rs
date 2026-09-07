use std::net::SocketAddr;
use tokio::sync::mpsc::Sender;

use capnp::message::ReaderOptions;
use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use futures::AsyncReadExt;
use tracing::{debug, error, info, instrument, warn};

use super::vmm::{Command, Factory, Handle, Reader, Registry};

#[derive(Clone)]
pub struct Server<F: Factory> {
    state: Registry<F, Reader>,
    factory: F,
    tx: Sender<Command<F>>,
    listen_addr: SocketAddr,
}

impl<F: Factory> Server<F> {
    pub fn new(
        state: Registry<F, Reader>,
        factory: F,
        tx: Sender<Command<F>>,
        listen_addr: SocketAddr,
    ) -> Self {
        Self {
            state,
            factory,
            tx,
            listen_addr,
        }
    }

    /// # Errors
    ///
    /// - if the TCP listener fails to bind to the given address
    /// - if the RPC system fails to start
    ///
    #[instrument(skip(self))]
    pub async fn serve(self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        info!(addr = %addr, "Starting worker's RPC server");
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .inspect_err(|err| {
                error!(addr = %addr, ?err, "Failed to bind worker TCP listener");
            })?;

        let client: commands::worker_capnp::worker::Client<F::BackendConfig> =
            capnp_rpc::new_client(self);

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

            let rpc_system = RpcSystem::new(Box::new(network), Some(client.clone().client));
            debug!(peer_addr = %peer_addr, "Spawning RPC system task for connection");
            tokio::task::spawn_local(rpc_system);
        }
    }
}

impl<F: Factory> commands::worker_capnp::worker::Server<F::BackendConfig> for Server<F> {
    fn create_vm(
        &mut self,
        request: commands::worker_capnp::worker::CreateVmParams<F::BackendConfig>,
        mut response: commands::worker_capnp::worker::CreateVmResults<F::BackendConfig>,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        let factory = self.factory.clone();
        let tx = self.tx.clone();

        capnp::capability::Promise::from_future(async move {
            debug!("Received create_vm RPC request");

            let vm_spec = request
                .get()
                .and_then(commands::worker_capnp::worker::create_vm_params::Reader::get_spec)
                .inspect_err(|err| {
                    error!(
                        ?err,
                        "Invalid create_vm RPC payload: missing or unreadable spec"
                    );
                })?;

            debug!("create_vm RPC payload parsed; converting backend VmSpec");
            let create_spec = vm_spec.try_into().inspect_err(|err| {
                error!(
                    ?err,
                    "Failed converting create_vm payload to backend CreateVmSpec"
                );
            })?;

            let msg = factory.create_vm(create_spec).await.inspect_err(|err| {
                error!(?err, "Factory failed to create VM command from spec");
            })?;

            let id = msg.id().to_string();

            debug!(id, "VM creation command created successfully");
            response.get().set_id(&id);

            // TODO: this could be an issue, and maybe we need a better way to handle the fact that sending a message with the process could fail
            // The current idea is to prepare, create and spawn the vm here.
            // Do we want to retry ourself or we let the user handle the retries?
            if tx.send(Command::Create(msg)).await.is_err() {
                error!(
                    vm_id = id,
                    "Failed to enqueue VM create command to supervisor"
                );
                return Err(capnp::Error::failed(
                    "Failed to send create command to node".into(),
                ));
            }

            info!(vm_id = id, "create_vm RPC handled successfully");
            Ok(())
        })
    }

    fn delete_vm(
        &mut self,
        request: commands::worker_capnp::worker::DeleteVmParams<F::BackendConfig>,
        _: commands::worker_capnp::worker::DeleteVmResults<F::BackendConfig>,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        let tx = self.tx.clone();
        let state = self.state.clone();

        capnp::capability::Promise::from_future(async move {
            debug!("Received delete_vm RPC request");
            let vm_id = request
                .get()
                .and_then(commands::worker_capnp::worker::delete_vm_params::Reader::get_id)
                .inspect_err(|err| {
                    error!(?err, "Invalid delete_vm RPC payload: missing id");
                })?;

            let vm_id = vm_id.to_string().map_err(|err| {
                error!(?err, "Invalid delete_vm id: non-UTF8 text");
                capnp::Error::failed(format!("invalid vm id utf8: {err}"))
            })?;

            debug!(vm_id = %vm_id, "delete_vm request parsed");
            if !state.exists(&vm_id).await {
                warn!(vm_id = %vm_id, "delete_vm requested unknown VM id");
                return Err(capnp::Error::failed(format!(
                    "VM with id {vm_id} doesn't exists"
                )));
            }
            // TODO: or we could save it in the sqlite database instead of sending a message
            if tx.send(Command::Delete(vm_id)).await.is_err() {
                error!("Failed to enqueue VM delete command to supervisor");
                return Err(capnp::Error::failed(
                    "Failed to send delete command to node".into(),
                ));
            }

            info!("delete_vm RPC handled successfully");
            Ok(())
        })
    }

    fn list_vms(
        &mut self,
        _: commands::worker_capnp::worker::ListVmsParams<F::BackendConfig>,
        mut response: commands::worker_capnp::worker::ListVmsResults<F::BackendConfig>,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        warn!("list_vms RPC called");
        let state = self.state.clone();
        let worker_id = self.listen_addr.to_string();
        capnp::capability::Promise::from_future(async move {
            let data = state.get().await;

            let mut vms = response
                .get()
                .init_vms(u32::try_from(data.len()).expect("len fits in u32"));

            for (index, (vm_id, handle)) in data.iter().enumerate() {
                let mut vm = vms
                    .reborrow()
                    .get(u32::try_from(index).expect("index fits in u32"));
                vm.set_id(vm_id);
                vm.set_worker_id(&worker_id);
                vm.set_desired_hash("");
                vm.set_observed_hash("");
                vm.set_ip(handle.ip());

                let status = if handle.health().await.is_ok() {
                    "running"
                } else {
                    "failed"
                };
                vm.set_status(status);
                vm.set_drifted(false);

                let mut metrics = vm.reborrow().init_metrics();
                metrics.set_cpu_usage(0.0);
                metrics.set_memory_usage(0);
                metrics.set_network_rx_bytes(0);
                metrics.set_network_tx_bytes(0);
            }

            Ok(())
        })
    }

    fn pause_vm(
        &mut self,
        request: commands::worker_capnp::worker::PauseVmParams<F::BackendConfig>,
        _: commands::worker_capnp::worker::PauseVmResults<F::BackendConfig>,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        let state = self.state.clone();
        capnp::capability::Promise::from_future(async move {
            let vm_id = request
                .get()
                .and_then(commands::worker_capnp::worker::pause_vm_params::Reader::get_id)
                .inspect_err(|err| error!(?err, "Invalid pause_vm RPC payload: missing id"))?
                .to_str()
                .map_err(|err| {
                    error!(?err, "Invalid pause_vm id: non-UTF8 text");
                    capnp::Error::failed(format!("invalid vm id utf8: {err}"))
                })?;

            debug!(vm_id, "pause_vm request parsed");

            let guard = state.get().await;
            let handle = guard.get(vm_id).ok_or_else(|| {
                warn!(vm_id, "pause_vm requested unknown VM id");
                capnp::Error::failed(format!("VM with id {vm_id} doesn't exist"))
            })?;

            handle.pause().await.map_err(|err| {
                error!(vm_id, ?err, "pause_vm failed");
                capnp::Error::failed(format!("pause failed: {err}"))
            })?;

            info!(vm_id, "pause_vm RPC handled successfully");
            Ok(())
        })
    }

    fn resume_vm(
        &mut self,
        request: commands::worker_capnp::worker::ResumeVmParams<F::BackendConfig>,
        _: commands::worker_capnp::worker::ResumeVmResults<F::BackendConfig>,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        let state = self.state.clone();
        capnp::capability::Promise::from_future(async move {
            let vm_id = request
                .get()
                .and_then(commands::worker_capnp::worker::resume_vm_params::Reader::get_id)
                .inspect_err(|err| error!(?err, "Invalid pause_vm RPC payload: missing id"))?
                .to_str()
                .map_err(|err| {
                    error!(?err, "Invalid pause_vm id: non-UTF8 text");
                    capnp::Error::failed(format!("invalid vm id utf8: {err}"))
                })?;

            debug!(vm_id, "resume_vm request parsed; spawning background task");

            let guard = state.get().await;
            let Some(handle) = guard.get(vm_id) else {
                warn!(vm_id, "resume_vm requested unknown VM id");
                return Ok(());
            };
            if let Err(err) = handle.resume().await {
                error!(vm_id, ?err, "background resume_vm failed");
                Err(capnp::Error::failed(format!(
                    "background resume failed: {err}"
                )))
            } else {
                info!(vm_id, "background resume_vm succeeded");
                Ok(())
            }
        })
    }

    fn snapshot_vm(
        &mut self,
        request: commands::worker_capnp::worker::SnapshotVmParams<F::BackendConfig>,
        mut response: commands::worker_capnp::worker::SnapshotVmResults<F::BackendConfig>,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        let state = self.state.clone();
        capnp::capability::Promise::from_future(async move {
            let params = request.get()?;
            let vm_id = params
                .get_id()
                .inspect_err(|err| error!(?err, "Invalid snapshot_vm RPC payload: missing id"))?
                .to_str()
                .map_err(|err| {
                    error!(?err, "Invalid snapshot_vm id: non-UTF8 text");
                    capnp::Error::failed(format!("invalid vm id utf8: {err}"))
                })?;

            let destination = params
                .get_destination()
                .inspect_err(|err| {
                    error!(?err, "Invalid snapshot_vm RPC payload: missing destination");
                })?
                .to_str()
                .map_err(|err| {
                    error!(?err, "Invalid snapshot_vm destination: non-UTF8 text");
                    capnp::Error::failed(format!("invalid destination utf8: {err}"))
                })?;

            if destination.is_empty() {
                return Err(capnp::Error::failed("destination must not be empty".into()));
            }

            debug!(vm_id, %destination, "snapshot_vm request parsed");

            let dest_path = std::path::PathBuf::from(destination);

            let guard = state.get().await;
            let handle = guard.get(vm_id).ok_or_else(|| {
                warn!(vm_id, "snapshot_vm requested unknown VM id");
                capnp::Error::failed(format!("VM with id {vm_id} doesn't exist"))
            })?;

            handle.snapshot(dest_path.clone()).await.map_err(|err| {
                error!(vm_id, ?err, "snapshot_vm failed");
                capnp::Error::failed(format!("snapshot failed: {err}"))
            })?;

            // Return the canonicalized absolute path when possible; fall back to the
            // raw destination if canonicalize fails (e.g. on a path the caller still
            // wants to reference even though it's not yet visible to the kernel).
            let resolved = tokio::fs::canonicalize(&dest_path)
                .await
                .unwrap_or(dest_path);
            let resolved_str = resolved.to_string_lossy();
            response.get().set_path(&resolved_str);

            info!(vm_id, path = %resolved_str, "snapshot_vm RPC handled successfully");
            Ok(())
        })
    }

    fn backup_disk(
        &mut self,
        request: commands::worker_capnp::worker::BackupDiskParams<F::BackendConfig>,
        mut response: commands::worker_capnp::worker::BackupDiskResults<F::BackendConfig>,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        let state = self.state.clone();
        capnp::capability::Promise::from_future(async move {
            let params = request.get()?;
            let vm_id = params
                .get_id()
                .inspect_err(|err| error!(?err, "Invalid backup_disk RPC payload: missing id"))?
                .to_str()
                .map_err(|err| {
                    error!(?err, "Invalid backup_disk id: non-UTF8 text");
                    capnp::Error::failed(format!("invalid vm id utf8: {err}"))
                })?;
            let destination = params
                .get_destination()
                .inspect_err(|err| {
                    error!(?err, "Invalid backup_disk RPC payload: missing destination");
                })?
                .to_str()
                .map_err(|err| {
                    error!(?err, "Invalid backup_disk destination: non-UTF8 text");
                    capnp::Error::failed(format!("invalid destination utf8: {err}"))
                })?;

            if destination.is_empty() {
                return Err(capnp::Error::failed("destination must not be empty".into()));
            }

            debug!(vm_id, %destination, "backup_disk request parsed");

            let dest_path = std::path::PathBuf::from(destination);

            let guard = state.get().await;
            let handle = guard.get(vm_id).ok_or_else(|| {
                warn!(vm_id, "backup_disk requested unknown VM id");
                capnp::Error::failed(format!("VM with id {vm_id} doesn't exist"))
            })?;

            handle.backup_disk(dest_path.clone()).await.map_err(|err| {
                error!(vm_id, ?err, "backup_disk failed");
                capnp::Error::failed(format!("backup failed: {err}"))
            })?;

            let resolved = tokio::fs::canonicalize(&dest_path)
                .await
                .unwrap_or(dest_path);
            let resolved_str = resolved.to_string_lossy();
            response.get().set_path(&resolved_str);

            info!(vm_id, path = %resolved_str, "backup_disk RPC handled successfully");
            Ok(())
        })
    }

    fn read(
        &mut self,
        _: commands::worker_capnp::worker::ReadParams<F::BackendConfig>,
        mut response: commands::worker_capnp::worker::ReadResults<F::BackendConfig>,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        debug!("read RPC called");
        let state = self.state.clone();
        let worker_id = self.listen_addr.to_string();
        capnp::capability::Promise::from_future(async move {
            let data = state.get().await;

            let mut worker = response.get().init_data();
            worker.set_id(&worker_id);

            let mut healthy = true;
            for (_, handle) in data.iter() {
                if handle.health().await.is_err() {
                    healthy = false;
                }
            }

            worker.set_healthy(healthy);
            worker.set_generation(0);
            worker.set_running_vms(u32::try_from(data.len()).expect("len fits in u32"));

            let mut resources = worker.reborrow().init_available_resources();
            resources.set_cpu(0.0);
            resources.set_memory_bytes(0);

            let mut metrics = worker.reborrow().init_metrics();
            metrics.set_available_cpu(0.0);
            metrics.set_available_memory(0);
            metrics.set_disk_usage(0);
            metrics.set_uptime(0);

            Ok(())
        })
    }
}
