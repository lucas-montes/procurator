//! Central point of communication. Talks to workers and receives requests from the cli.
use std::net::SocketAddr;

use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use futures::AsyncReadExt;
use tracing::{debug, info, instrument};

use crate::dto::NodeMessenger;

#[derive(Clone)]
pub struct Server {
    messenger: NodeMessenger,
}

impl Server {
    pub fn new(messenger: impl Into<NodeMessenger>) -> Self {
        Server {
            messenger: messenger.into(),
        }
    }

    #[instrument(skip(self))]
    pub async fn serve(self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        info!(addr = %addr, "Starting server");
        let listener = tokio::net::TcpListener::bind(&addr).await?;

        let client: commands::master_capnp::master::Client = capnp_rpc::new_client(self);

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
                Default::default(),
            );

            // TODO: Determine which client to provide based on connection context
            // For now, defaulting to master_control for CLI connections
            let rpc_system = RpcSystem::new(Box::new(network), Some(client.clone().client));

            tokio::task::spawn_local(rpc_system);
        }
    }
}

impl commands::master_capnp::master::Server for Server {
    fn publish_state(
        &mut self,
        params: commands::master_capnp::master::PublishStateParams,
        mut results: commands::master_capnp::master::PublishStateResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        match params.get() {
            Ok(p) => {
                let commit = p.get_commit();
                let generation = p.get_generation();
                let intent_hash = p.get_intent_hash();
                let _vm_specs = p.get_vm_specs();

                info!(generation, ?commit, ?intent_hash, "Publish request");

                // TODO: Implement actual publishing logic
                if let Ok(result_builder) = results.get().get_result() {
                    let _ = result_builder.init_ok();
                }

                ::capnp::capability::Promise::ok(())
            }
            Err(e) => ::capnp::capability::Promise::err(e),
        }
    }

    fn get_assignment(
        &mut self,
        params: commands::master_capnp::master::GetAssignmentParams,
        mut results: commands::master_capnp::master::GetAssignmentResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        match params.get() {
            Ok(p) => {
                let worker_id = p.get_worker_id();
                let last_seen_generation = p.get_last_seen_generation();

                debug!(?worker_id, last_seen_generation, "Getting assignment");

                // TODO: Implement assignment retrieval
                if let Ok(mut result_builder) = results.get().get_result() {
                    let _ = result_builder.set_err("not implemented");
                }

                ::capnp::capability::Promise::ok(())
            }
            Err(e) => ::capnp::capability::Promise::err(e),
        }
    }

    fn push_data(
        &mut self,
        params: commands::master_capnp::master::PushDataParams,
        mut results: commands::master_capnp::master::PushDataResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        match params.get() {
            Ok(p) => {
                let worker_id = p.get_worker_id();
                let observed_generation = p.get_observed_generation();
                let _running_vms = p.get_running_vms();
                let _metrics = p.get_metrics();

                debug!(?worker_id, observed_generation, "Worker pushing data");

                // TODO: Implement state observation logic
                if let Ok(result_builder) = results.get().get_result() {
                    let _ = result_builder.init_ok();
                }

                ::capnp::capability::Promise::ok(())
            }
            Err(e) => ::capnp::capability::Promise::err(e),
        }
    }

    fn get_cluster_status(
        &mut self,
        _params: commands::master_capnp::master::GetClusterStatusParams,
        _results: commands::master_capnp::master::GetClusterStatusResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        debug!("Getting cluster status");
        // TODO: Implement cluster status retrieval
        ::capnp::capability::Promise::ok(())
    }

    fn get_worker(
        &mut self,
        params: commands::master_capnp::master::GetWorkerParams,
        _results: commands::master_capnp::master::GetWorkerResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        match params.get() {
            Ok(p) => {
                let worker_id = p.get_worker_id();
                debug!(?worker_id, "Getting worker capability");

                // TODO: Lookup worker capability from registered workers
                // For now, this is unimplemented - need to store worker capabilities when they connect
                let error = capnp::Error::failed("Worker lookup not yet implemented".to_string());
                ::capnp::capability::Promise::err(error)
            }
            Err(e) => ::capnp::capability::Promise::err(e),
        }
    }
}
