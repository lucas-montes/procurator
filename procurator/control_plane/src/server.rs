//! Central point of communication. Talks to workers and receives requests from the cli.
use std::net::SocketAddr;

use capnp_rpc::{RpcSystem, pry, rpc_twoparty_capnp, twoparty};
use commands::control_plane;
use futures::AsyncReadExt;

use crate::dto::{NodeMessage, NodeMessenger};

pub struct Server {
    node_channel: NodeMessenger,
}

impl Server {
    pub fn new(node_channel: impl Into<NodeMessenger>) -> Self {
        Server {
            node_channel: node_channel.into(),
        }
    }

    pub async fn serve(self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        let client: control_plane::Client = capnp_rpc::new_client(self);
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

            let rpc_system = RpcSystem::new(Box::new(network), Some(client.clone().client));

            tokio::task::spawn_local(rpc_system);
        }
    }
}

impl control_plane::Server for Server {
    /// Handles the `apply` command, which applies changes to a file
    fn apply(
        &mut self,
        params: control_plane::ApplyParams,
        mut results: control_plane::ApplyResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        let file = pry!(pry!(pry!(params.get()).get_file()).to_string());
        let name = pry!(pry!(pry!(params.get()).get_name()).to_string());
        let channel = self.node_channel.clone();
        capnp::capability::Promise::from_future(async move {
            let mut tx = channel.apply(file, name).await;
            match tx.try_recv() {
                Ok(msg) => {
                    let mut resp = results.get().init_response();
                    match msg {
                        Ok(_) => {
                            resp.set_ok(());
                        }
                        Err(err) => {
                            resp.set_err(err);
                        }
                    }
                    Ok(())
                }
                Err(err) => Err(capnp::Error::failed("wrong".into())),
            }
        })
    }

    //TODO: I should look into the stream feature
    fn monitor(
        &mut self,
        params: control_plane::MonitorParams,
        mut results: control_plane::MonitorResults,
    ) -> capnp::capability::Promise<(), capnp::Error> {
        capnp::capability::Promise::ok(())
    }
}
