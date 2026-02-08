//! Worker node server. Receives commands from master and exposes VM interaction capabilities.
use std::net::SocketAddr;

use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use futures::AsyncReadExt;
use tokio_util::compat::TokioAsyncReadCompatExt;

use crate::dto::WorkerMessenger;

pub struct Server {
    messenger: WorkerMessenger,
}

impl Server {
    pub fn new(messenger: impl Into<WorkerMessenger>) -> Self {
        Server {
            messenger: messenger.into(),
        }
    }

    pub async fn serve(self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        // Worker implements WorkerControl directly
        let client: commands::worker_control::Client = capnp_rpc::new_client(WorkerControlImpl {
            messenger: self.messenger.clone(),
        });
        loop {
            let (stream, _) = listener.accept().await?;
            stream.set_nodelay(true)?;
            let (reader, writer) =
                TokioAsyncReadCompatExt::compat(stream).split();
            let network = twoparty::VatNetwork::new(
                futures::io::BufReader::new(reader),
                futures::io::BufWriter::new(writer),
                rpc_twoparty_capnp::Side::Server,
                Default::default(),
            );

            let rpc_system = RpcSystem::new(Box::new(network), Some(client.clone().client));

            tokio::task::spawn_local(rpc_system);
        }
    }
}

// ============================================================================
// Worker Control Implementation
// ============================================================================

struct WorkerControlImpl {
    messenger: WorkerMessenger,
}

impl commands::worker_control::Server for WorkerControlImpl {
    fn get_assignment(
        &mut self,
        _params: commands::worker_control::GetAssignmentParams,
        _results: commands::worker_control::GetAssignmentResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        // TODO: Use messenger to request assignment from Node
        capnp::capability::Promise::err(capnp::Error::unimplemented("not implemented".into()))
    }

    fn push_observed_state(
        &mut self,
        _params: commands::worker_control::PushObservedStateParams,
        _results: commands::worker_control::PushObservedStateResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        // TODO: Use messenger to push state to Node
        capnp::capability::Promise::err(capnp::Error::unimplemented("not implemented".into()))
    }
}
