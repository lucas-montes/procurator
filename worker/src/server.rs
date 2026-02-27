//! Stateless RPC adapter. Translates Cap'n Proto calls into VmCommands,
//! sends them to the Node via mpsc channel, and fills responses from oneshot replies.
//!
//! Holds only a `CommandSender` (cloneable mpsc::Sender wrapper). No VMM, no VM state.

use std::net::SocketAddr;

use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use futures::AsyncReadExt;
use tracing::{debug, info, instrument};

use crate::dto::{CommandSender, CommandPayload, CommandResponse, VmSpec};

#[derive(Clone)]
pub struct Server {
    tx: CommandSender,
}

impl Server {
    pub fn new(tx: CommandSender) -> Self {
        Server { tx }
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

        let tx = self.tx.clone();
        ::capnp::capability::Promise::from_future(async move {
            let resp = tx.request(CommandPayload::GetWorkerStatus).await
                .map_err(|e| capnp::Error::failed(e.to_string()))?;

            if let CommandResponse::WorkerInfo(info) = resp {
                if let Ok(mut data) = results.get().get_data() {
                    data.set_id(info.id());
                    data.set_healthy(info.healthy());
                    data.set_generation(info.generation());
                    data.set_running_vms(info.running_vms());
                }
            } else {
                return Err(capnp::Error::failed("unexpected response for GetWorkerStatus".to_string()));
            }

            Ok(())
        })
    }

    fn list_vms(
        &mut self,
        _params: commands::worker_capnp::worker::ListVmsParams,
        mut results: commands::worker_capnp::worker::ListVmsResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        debug!("Worker.list_vms called");

        let tx = self.tx.clone();
        ::capnp::capability::Promise::from_future(async move {
            let resp = tx.request(CommandPayload::List).await
                .map_err(|e| capnp::Error::failed(e.to_string()))?;

            if let CommandResponse::VmList(vm_infos) = resp {
                let mut vms = results.get().init_vms(vm_infos.len() as u32);
                for (i, info) in vm_infos.iter().enumerate() {
                let mut vm_status = vms.reborrow().get(i as u32);
                vm_status.set_id(info.id());
                vm_status.set_worker_id(info.worker_id());
                vm_status.set_desired_hash(info.desired_hash());
                vm_status.set_observed_hash(info.observed_hash());
                vm_status.set_status(info.status().as_str());
                vm_status.set_drifted(info.status().is_drifted(
                    info.desired_hash(),
                    info.observed_hash(),
                ));
                let mut metrics = vm_status.init_metrics();
                metrics.set_cpu_usage(info.metrics().cpu_usage);
                metrics.set_memory_usage(info.metrics().memory_usage);
                metrics.set_network_rx_bytes(info.metrics().network_rx_bytes);
                metrics.set_network_tx_bytes(info.metrics().network_tx_bytes);
            }
            } else {
                return Err(capnp::Error::failed("unexpected response for List".to_string()));
            }

            Ok(())
        })
    }

    fn create_vm(
        &mut self,
        params: commands::worker_capnp::worker::CreateVmParams,
        mut results: commands::worker_capnp::worker::CreateVmResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        debug!("Worker.create_vm called");

        let tx = self.tx.clone();
        ::capnp::capability::Promise::from_future(async move {
            let spec_reader = params.get()?.get_spec()?;

            let mut domains = Vec::new();
            for d in spec_reader.get_network_allowed_domains()? {
                domains.push(d?.to_str()
                    .map_err(|e| capnp::Error::failed(e.to_string()))?
                    .to_string());
            }

            let to_string = |r: capnp::text::Reader<'_>| -> Result<String, capnp::Error> {
                r.to_str()
                    .map(|s| s.to_string())
                    .map_err(|e| capnp::Error::failed(e.to_string()))
            };

            let spec = VmSpec::new(
                to_string(spec_reader.get_toplevel()?)?,
                to_string(spec_reader.get_kernel_path()?)?,
                to_string(spec_reader.get_initrd_path()?)?,
                to_string(spec_reader.get_disk_image_path()?)?,
                to_string(spec_reader.get_cmdline()?)?,
                spec_reader.get_cpu(),
                spec_reader.get_memory_mb(),
                domains,
            );

            let resp = tx.request(CommandPayload::Create(spec)).await
                .map_err(|e| capnp::Error::failed(e.to_string()))?;

            if let CommandResponse::VmId(id) = resp {
                results.get().set_id(&id);
            } else {
                return Err(capnp::Error::failed("unexpected response for Create".into()));
            }

            Ok(())
        })
    }

    fn delete_vm(
        &mut self,
        params: commands::worker_capnp::worker::DeleteVmParams,
        _results: commands::worker_capnp::worker::DeleteVmResults,
    ) -> ::capnp::capability::Promise<(), ::capnp::Error> {
        debug!("Worker.delete_vm called");

        let tx = self.tx.clone();
        ::capnp::capability::Promise::from_future(async move {
            let id = params.get()?.get_id()?
                .to_str()
                .map_err(|e| capnp::Error::failed(e.to_string()))?
                .to_string();

            let resp = tx.request(CommandPayload::Delete(id)).await
                .map_err(|e| capnp::Error::failed(e.to_string()))?;

            if let CommandResponse::Unit = resp {
                Ok(())
            } else {
                Err(capnp::Error::failed("unexpected response for Delete".into()))
            }
        })
    }

}
