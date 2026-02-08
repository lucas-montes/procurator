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
                Default::default(),
            );

            // TODO: Determine which client to provide based on connection context
            // For now, defaulting to master_control for CLI connections
            let rpc_system = RpcSystem::new(Box::new(network), Some(client.clone().client));

            tokio::task::spawn_local(rpc_system);
        }
    }
}

impl commands::worker_capnp::worker::Server for Server {
    fn read(
        &mut self,
        _params: commands::worker_capnp::worker::ReadParams,
        mut results: commands::worker_capnp::worker::ReadResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        debug!("Worker.read called");

        if let Ok(mut data) = results.get().get_data() {
            data.set_id("worker-unknown");
            data.set_healthy(false);
            data.set_generation(0);
            data.set_running_vms(0);
            // data.init_available_resources();
            // data.init_metrics();
        }

        ::capnp::capability::Promise::ok(())
    }

    fn list_vms(
        &mut self,
        _params: commands::worker_capnp::worker::ListVmsParams,
        mut results: commands::worker_capnp::worker::ListVmsResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        debug!("Worker.list_vms called");

        let _ = results.get().init_vms(0);
        ::capnp::capability::Promise::ok(())
    }

    fn get_vm(
        &mut self,
        params: commands::worker_capnp::worker::GetVmParams,
        mut results: commands::worker_capnp::worker::GetVmResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        match params.get() {
            Ok(p) => {
                let vm_id = p.get_vm_id();
                debug!(?vm_id, "Worker.get_vm called");

                results
                    .get()
                    .set_vm(capnp_rpc::new_client(self.clone()));

                ::capnp::capability::Promise::ok(())
            }
            Err(e) => ::capnp::capability::Promise::err(e),
        }
    }
}

impl commands::worker_capnp::worker::vm::Server for Server {
    fn read(
        &mut self,
        _params: commands::worker_capnp::worker::vm::ReadParams,
        mut results: commands::worker_capnp::worker::vm::ReadResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        debug!("Worker.Vm.read called");

        if let Ok(mut data) = results.get().get_data() {
            data.set_id("vm-unknown");
            data.set_worker_id("worker-unknown");
            data.set_desired_hash("");
            data.set_observed_hash("");
            data.set_status("unknown");
            data.set_drifted(false);
            data.init_metrics();
        }

        ::capnp::capability::Promise::ok(())
    }

    fn get_logs(
        &mut self,
        _params: commands::worker_capnp::worker::vm::GetLogsParams,
        mut results: commands::worker_capnp::worker::vm::GetLogsResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        debug!("Worker.Vm.get_logs called");

        if let Ok(mut logs) = results.get().get_logs() {
            logs.set_logs("");
            logs.set_truncated(false);
        }

        ::capnp::capability::Promise::ok(())
    }

    fn exec(
        &mut self,
        _params: commands::worker_capnp::worker::vm::ExecParams,
        mut results: commands::worker_capnp::worker::vm::ExecResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        debug!("Worker.Vm.exec called");

        if let Ok(mut output) = results.get().get_output() {
            output.set_stdout("");
            output.set_stderr("not implemented");
            output.set_exit_code(1);
        }

        ::capnp::capability::Promise::ok(())
    }

    fn get_connection_info(
        &mut self,
        _params: commands::worker_capnp::worker::vm::GetConnectionInfoParams,
        mut results: commands::worker_capnp::worker::vm::GetConnectionInfoResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        debug!("Worker.Vm.get_connection_info called");

        if let Ok(mut info) = results.get().get_info() {
            info.set_vm_id("vm-unknown");
            info.set_worker_host("127.0.0.1");
            info.set_ssh_port(0);
            info.set_console_port(0);
            info.set_username("root");
        }

        ::capnp::capability::Promise::ok(())
    }
}
