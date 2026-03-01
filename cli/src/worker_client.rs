//! Cap'n Proto RPC client for the Worker interface.
//!
//! Provides connect + one function per RPC method defined in worker.capnp.

use capnp_rpc::{rpc_twoparty_capnp, twoparty, RpcSystem};
use commands::worker_capnp;
use futures::AsyncReadExt;
use std::net::SocketAddr;
use tracing::info;

use crate::VmSpecJson;

pub type WorkerClient = worker_capnp::worker::Client;

/// Connect to a running Worker server and return the bootstrap capability.
pub async fn connect(addr: SocketAddr) -> Result<WorkerClient, Box<dyn std::error::Error>> {
    info!(addr = %addr, "Connecting to Worker server");

    let stream = tokio::net::TcpStream::connect(&addr).await?;
    stream.set_nodelay(true)?;

    let (reader, writer) = tokio_util::compat::TokioAsyncReadCompatExt::compat(stream).split();
    let network = Box::new(twoparty::VatNetwork::new(
        futures::io::BufReader::new(reader),
        futures::io::BufWriter::new(writer),
        rpc_twoparty_capnp::Side::Client,
        Default::default(),
    ));

    let mut rpc_system = RpcSystem::new(network, None);
    let client: WorkerClient = rpc_system.bootstrap(rpc_twoparty_capnp::Side::Server);

    tokio::task::spawn_local(rpc_system);

    info!("Connected successfully");
    Ok(client)
}

/// Worker.read — fetch worker status.
pub async fn read(client: &WorkerClient) -> Result<(), Box<dyn std::error::Error>> {
    info!("Worker.read()");

    let response = client.read_request().send().promise.await?;
    let data = response.get()?.get_data()?;

    let id = data.get_id()?.to_str()?;
    let healthy = data.get_healthy();
    let generation = data.get_generation();
    let running_vms = data.get_running_vms();

    info!(
        id = %id,
        healthy = healthy,
        generation = generation,
        running_vms = running_vms,
        "✓ Worker status"
    );

    Ok(())
}

/// Worker.listVms — list all managed VMs.
pub async fn list_vms(client: &WorkerClient) -> Result<(), Box<dyn std::error::Error>> {
    info!("Worker.listVms()");

    let response = client.list_vms_request().send().promise.await?;
    let vms = response.get()?.get_vms()?;

    if vms.is_empty() {
        info!("✓ No VMs running");
        return Ok(());
    }

    info!(count = vms.len(), "✓ VMs listed");
    for i in 0..vms.len() {
        let vm = vms.get(i);
        let id = vm.get_id()?.to_str()?;
        let status = vm.get_status()?.to_str()?;
        let drifted = vm.get_drifted();
        let metrics = vm.get_metrics()?;
        info!(
            id = %id,
            status = %status,
            drifted = drifted,
            cpu = metrics.get_cpu_usage(),
            memory_bytes = metrics.get_memory_usage(),
            "  VM"
        );
    }

    Ok(())
}

/// Worker.createVm — create a VM from a spec.
pub async fn create_vm(
    client: &WorkerClient,
    spec: VmSpecJson,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(
        kernel = %spec.kernel_path,
        disk = %spec.disk_image_path,
        cpu = spec.cpu,
        memory_mb = spec.memory_mb,
        "Worker.createVm()"
    );

    let mut request = client.create_vm_request();
    {
        let mut s = request.get().init_spec();
        s.set_toplevel(&spec.toplevel);
        s.set_kernel_path(&spec.kernel_path);
        s.set_initrd_path(&spec.initrd_path);
        s.set_disk_image_path(&spec.disk_image_path);
        s.set_cmdline(&spec.cmdline);
        s.set_cpu(spec.cpu);
        s.set_memory_mb(spec.memory_mb);
        let mut domains = s.init_network_allowed_domains(spec.network_allowed_domains.len() as u32);
        for (i, d) in spec.network_allowed_domains.iter().enumerate() {
            domains.set(i as u32, d);
        }
    }

    let response = request.send().promise.await?;
    let id = response.get()?.get_id()?.to_str()?;

    info!(id = %id, "✓ VM created");
    Ok(())
}

/// Worker.deleteVm — delete a VM by ID.
pub async fn delete_vm(
    client: &WorkerClient,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(id = %id, "Worker.deleteVm()");

    let mut request = client.delete_vm_request();
    request.get().set_id(id);

    request.send().promise.await?;

    info!(id = %id, "✓ VM deleted");
    Ok(())
}
