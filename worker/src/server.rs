use std::net::SocketAddr;
use tokio::sync::mpsc::Sender;

use capnp::message::ReaderOptions;
use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use futures::AsyncReadExt;
use tracing::{debug, error, info, instrument, warn};

use super::vmm::{Command, Factory, Reader, Registry};

#[derive(Clone)]
pub struct Server<F: Factory> {
    state: Registry<F, Reader>,
    factory: F,
    tx: Sender<Command<F>>,
}

impl<F: Factory> Server<F> {
    pub fn new(state: Registry<F, Reader>, factory: F, tx: Sender<Command<F>>) -> Self {
        Self { state, factory, tx }
    }

    /// # Errors
    ///
    /// - if the TCP listener fails to bind to the given address
    /// - if the RPC system fails to start
    ///
    #[instrument(skip(self))]
    pub async fn serve(self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        info!(addr = %addr, "Starting server");
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
                .and_then(|r| r.get_spec())
                .inspect_err(|err| {
                    error!(?err, "Invalid create_vm RPC payload: missing or unreadable spec");
                })?;

            debug!("create_vm RPC payload parsed; converting backend VmSpec");
            let create_spec = vm_spec.try_into().inspect_err(|err| {
                error!(?err, "Failed converting create_vm payload to backend CreateVmSpec");
            })?;

            let msg = factory
                .create_vm(create_spec)
                .await
                .inspect_err(|err| {
                    error!(?err, "Factory failed to create VM command from spec");
                })?;

            let id = msg.id().to_string();

            debug!(id, "VM creation command created successfully");
            response.get().set_id(&id);

            // TODO: this could be an issue, and maybe we need a better way to handle the fact that sending a message with the process could fail
            // The current idea is to prepare, create and spawn the vm here.
            // Do we want to retry ourself or we let the user handle the retries?
            if tx.send(Command::Create(msg)).await.is_err() {
                error!(vm_id = id, "Failed to enqueue VM create command to supervisor");
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
                .and_then(|r| r.get_id())
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
        _: commands::worker_capnp::worker::ListVmsResults<F::BackendConfig>,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        warn!("list_vms RPC called but not implemented yet; returning empty success");
        capnp::capability::Promise::ok(())
    }

    fn read(
        &mut self,
        _: commands::worker_capnp::worker::ReadParams<F::BackendConfig>,
        _: commands::worker_capnp::worker::ReadResults<F::BackendConfig>,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        warn!("read RPC called but not implemented yet; returning empty success");
        capnp::capability::Promise::ok(())
    }
}
