use std::net::SocketAddr;
use tokio::sync::mpsc::Sender;

use capnp::message::ReaderOptions;
use capnp_rpc::{RpcSystem, pry, rpc_twoparty_capnp, twoparty};
use futures::AsyncReadExt;
use tracing::{debug, info, instrument};

use super::vmm::{Command, Factory, Reader, Registry, VmSpecRef};

#[derive(Clone)]
pub struct Server<F: Factory + Clone + 'static> {
    state: Registry<F, Reader>,
    factory: F,
    tx: Sender<Command<F>>,
}

impl<F: Factory + Clone + 'static> Server<F> {
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

        let client: commands::worker_capnp::worker::Client = capnp_rpc::new_client(self);

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

impl<F: Factory + Clone + 'static> commands::worker_capnp::worker::Server for Server<F> {
    fn create_vm(
        &mut self,
        request: commands::worker_capnp::worker::CreateVmParams,
        mut response: commands::worker_capnp::worker::CreateVmResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        let factory = self.factory.clone();
        let tx = self.tx.clone();

        capnp::capability::Promise::from_future(async move {
            let vm_spec = request.get()?.get_spec()?;

            let spec = VmSpecRef::try_from(vm_spec)?;

            let msg = factory
                .create_vm(spec)
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
        request: commands::worker_capnp::worker::DeleteVmParams,
        _: commands::worker_capnp::worker::DeleteVmResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        let factory = self.factory.clone();
        let tx = self.tx.clone();

        capnp::capability::Promise::from_future(async move {
            let vm_id = request.get()?.get_id()?.to_str()?;
            if let Err(err) = factory.delete_vm(vm_id).await {
                return Err(capnp::Error::failed(format!(
                    "Failed to delete VM: {:?}",
                    err
                )));
            };
            Ok(())
        })
    }

    fn list_vms(
        &mut self,
        _: commands::worker_capnp::worker::ListVmsParams,
        _: commands::worker_capnp::worker::ListVmsResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        capnp::capability::Promise::ok(())
    }

    fn read(
        &mut self,
        _: commands::worker_capnp::worker::ReadParams,
        _: commands::worker_capnp::worker::ReadResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        capnp::capability::Promise::ok(())
    }
}
