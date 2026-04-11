use std::{net::SocketAddr, sync::mpsc::Sender};

use capnp::message::ReaderOptions;
use capnp_rpc::{RpcSystem, pry, rpc_twoparty_capnp, twoparty};
use futures::AsyncReadExt;
use tracing::{debug, info, instrument};


use super::vmm::{Command,VmSpecRef, Factory, Reader, Registry};

#[derive(Clone)]
pub struct Server<F: Factory + Clone + 'static> {
    state: Registry<F, Reader>,
    factory: F,
    tx: Sender<Command>,
}

impl<F: Factory + Clone + 'static> Server<F> {
    pub fn new(state: Registry<F, Reader>, factory: F, tx: Sender<Command>) -> Self {
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
        let vm_spec = pry!(pry!(request.get()).get_spec());

        let spec = pry!(VmSpecRef::try_from(vm_spec));

        self.factory.

        debug!(vm_id = %vm_id, "Received create_vm request");
        if self.tx.send(Command::Create).is_err() {
            return capnp::capability::Promise::err(capnp::Error::failed(
                "Failed to send create command to node".into(),
            ));
        }

        response.get().set_id(id);

        capnp::capability::Promise::ok(())
    }

    fn delete_vm(
        &mut self,
        request: commands::worker_capnp::worker::DeleteVmParams,
        _: commands::worker_capnp::worker::DeleteVmResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        let vm_id = pry!(pry!(request.get()).get_id());
        capnp::capability::Promise::ok(())
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
