//! Central point of communication. Talks to workers and receives requests from the cli.
use std::net::SocketAddr;

use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use futures::AsyncReadExt;

use crate::dto::NodeMessenger;

pub struct Server {
    messenger: NodeMessenger,
}

impl Server {
    pub fn new(messenger: impl Into<NodeMessenger>) -> Self {
        Server {
            messenger: messenger.into(),
        }
    }

    pub async fn serve(self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        // Server implements all three interfaces directly
        let publisher_client: commands::desired_state_publisher::Client = capnp_rpc::new_client(PublisherImpl {
            messenger: self.messenger.clone(),
        });
        let master_client: commands::master_control::Client = capnp_rpc::new_client(MasterControlImpl {
            messenger: self.messenger.clone(),
        });
        let worker_client: commands::worker_control::Client = capnp_rpc::new_client(WorkerControlImpl {
            messenger: self.messenger.clone(),
        });

        loop {
            let (stream, _) = listener.accept().await?;
            stream.set_nodelay(true)?;
            let (reader, writer) =
                tokio_util::compat::TokioAsyncReadCompatExt::compat(stream).split();
            let network = twoparty::VatNetwork::new(
                futures::io::BufReader::new(reader),
                futures::io::BufWriter::new(writer),
                rpc_twoparty_capnp::Side::Server,
                Default::default(),
            );

            // TODO: Determine which client to provide based on connection context
            // For now, defaulting to master_control for CLI connections
            let rpc_system = RpcSystem::new(Box::new(network), Some(master_client.clone().client));

            tokio::task::spawn_local(rpc_system);
        }
    }
}

// ============================================================================
// Interface Implementations
// ============================================================================

struct PublisherImpl {
    messenger: NodeMessenger,
}

impl commands::desired_state_publisher::Server for PublisherImpl {
    fn publish_desired_state(
        &mut self,
        _params: commands::desired_state_publisher::PublishDesiredStateParams,
        _results: commands::desired_state_publisher::PublishDesiredStateResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        // TODO: Use messenger to send to Node
        capnp::capability::Promise::err(capnp::Error::unimplemented("not implemented".into()))
    }
}

// ============================================================================
// Sub-server implementations
// ============================================================================


struct MasterControlImpl {
    messenger: NodeMessenger,
}

impl commands::master_control::Server for MasterControlImpl {
    fn get_cluster_status(
        &mut self,
        _params: commands::master_control::GetClusterStatusParams,
        _results: commands::master_control::GetClusterStatusResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        // TODO: Use messenger to send to Node
        capnp::capability::Promise::err(capnp::Error::unimplemented("not implemented".into()))
    }

    fn get_worker(
        &mut self,
        _params: commands::master_control::GetWorkerParams,
        _results: commands::master_control::GetWorkerResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        // TODO: Use messenger to send to Node
        capnp::capability::Promise::err(capnp::Error::unimplemented("not implemented".into()))
    }

    fn get_vm(
        &mut self,
        _params: commands::master_control::GetVmParams,
        _results: commands::master_control::GetVmResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        // TODO: Use messenger to send to Node
        capnp::capability::Promise::err(capnp::Error::unimplemented("not implemented".into()))
    }
}

struct WorkerControlImpl {
    messenger: NodeMessenger,
}

impl commands::worker_control::Server for WorkerControlImpl {
    fn get_assignment(
        &mut self,
        _params: commands::worker_control::GetAssignmentParams,
        _results: commands::worker_control::GetAssignmentResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        // TODO: Use messenger to send to Node
        capnp::capability::Promise::err(capnp::Error::unimplemented("not implemented".into()))
    }

    fn push_observed_state(
        &mut self,
        _params: commands::worker_control::PushObservedStateParams,
        _results: commands::worker_control::PushObservedStateResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        // TODO: Use messenger to send to Node
        capnp::capability::Promise::err(capnp::Error::unimplemented("not implemented".into()))
    }
}
