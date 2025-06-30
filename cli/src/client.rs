use std::net::SocketAddr;

use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use commands::control_plane;
use futures::AsyncReadExt;

use crate::commands::CliError;

pub struct Client(control_plane::Client);

impl Client {
    pub async fn new(addr: SocketAddr) -> Self {
        let stream = tokio::net::TcpStream::connect(addr)
            .await
            .expect("couldn't connect to stream");
        stream.set_nodelay(true).expect("stream set_nodelay failed");
        let (reader, writer) = tokio_util::compat::TokioAsyncReadCompatExt::compat(stream).split();
        let rpc_network = Box::new(twoparty::VatNetwork::new(
            futures::io::BufReader::new(reader),
            futures::io::BufWriter::new(writer),
            rpc_twoparty_capnp::Side::Client,
            Default::default(),
        ));
        let mut rpc_system = RpcSystem::new(rpc_network, None);
        let client: control_plane::Client = rpc_system.bootstrap(rpc_twoparty_capnp::Side::Server);

        tokio::task::spawn_local(rpc_system);
        Self(client)
    }

    pub async fn apply(&self, config_file: &str) -> Result<(), CliError> {
        let mut req = self.0.apply_request();
        req.get().set_file(config_file);
        let response = req
            .send()
            .promise
            .await
            .map_err(|err| CliError::RequestFailed(err.to_string()))?;
        match response
            .get()
            .unwrap()
            .get_response()
            .unwrap()
            .which()
            .unwrap()
        {
            commands::apply_response::Which::Ok(()) => Ok(()),
            commands::apply_response::Which::Err(err) => {
                Err(CliError::RequestFailed(err.unwrap().to_string().unwrap()))
            }
        }
    }

    pub async fn monitor(&self) -> Result<(), CliError> {
        let req = self.0.monitor_request();
        let response = req
            .send()
            .promise
            .await
            .map_err(|err| CliError::RequestFailed(err.to_string()))?;
        response.get().unwrap().get_response().unwrap();
        Ok(())
    }
}
