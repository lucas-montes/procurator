use std::net::SocketAddr;
use tokio::sync::mpsc::Sender;

use capnp::message::ReaderOptions;
use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use futures::AsyncReadExt;
use tracing::{debug, info, instrument};

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
        let listener = tokio::net::TcpListener::bind(&addr).await?;

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
            let vm_spec = request.get()?.get_spec()?;
            let msg = factory
                .create_vm(vm_spec.try_into()?)
                .await
                .map_err(|e| capnp::Error::failed(format!("Failed to create vm {e:?}")))?;

            let id = msg.id();

            tracing::debug!(id, "VM creation command created successfully");
            response.get().set_id(id);

            // TODO: this could be an issue, and maybe we need a better way to handle the fact that sending a message with the process could fail
            // The current idea is to prepare, create and spawn the vm here.
            // Do we want to retry ourself or we let the user handle the retries?
            if tx.send(Command::Create(msg)).await.is_err() {
                return Err(capnp::Error::failed(
                    "Failed to send create command to node".into(),
                ));
            }
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
            let vm_id = request.get()?.get_id()?.to_string()?;
            if !state.exists(&vm_id).await {
                return Err(capnp::Error::failed(format!(
                    "VM with id {vm_id} doesn't exists"
                )));
            }
            // TODO: or we could save it in the sqlite database instead of sending a message
            if tx.send(Command::Delete(vm_id)).await.is_err() {
                return Err(capnp::Error::failed(
                    "Failed to send delete command to node".into(),
                ));
            }
            Ok(())
        })
    }

    fn list_vms(
        &mut self,
        _: commands::worker_capnp::worker::ListVmsParams<F::BackendConfig>,
        _: commands::worker_capnp::worker::ListVmsResults<F::BackendConfig>,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        capnp::capability::Promise::ok(())
    }

    fn read(
        &mut self,
        _: commands::worker_capnp::worker::ReadParams<F::BackendConfig>,
        _: commands::worker_capnp::worker::ReadResults<F::BackendConfig>,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        capnp::capability::Promise::ok(())
    }
}
