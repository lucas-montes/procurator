//! Client functions for testing Worker server interfaces

use capnp_rpc::{rpc_twoparty_capnp, twoparty, RpcSystem};
use commands::worker_capnp;
use futures::AsyncReadExt;
use std::net::SocketAddr;
use tracing::info;

async fn connect_to_worker(
    addr: SocketAddr,
) -> Result<worker_capnp::worker::Client, Box<dyn std::error::Error>> {
    info!(addr = %addr, "Connecting to Worker server");

    let stream = tokio::net::TcpStream::connect(&addr).await?;
    stream.set_nodelay(true)?;

    let (reader, writer) = tokio_util::compat::TokioAsyncReadCompatExt::compat(stream).split();
    let rpc_network = Box::new(twoparty::VatNetwork::new(
        futures::io::BufReader::new(reader),
        futures::io::BufWriter::new(writer),
        rpc_twoparty_capnp::Side::Client,
        Default::default(),
    ));

    let mut rpc_system = RpcSystem::new(rpc_network, None);
    let client: worker_capnp::worker::Client =
        rpc_system.bootstrap(rpc_twoparty_capnp::Side::Server);

    tokio::task::spawn_local(rpc_system);

    info!("Connected successfully");
    Ok(client)
}

pub async fn test_worker_read(addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing Worker.read()");

    let worker = connect_to_worker(addr).await?;
    let request = worker.read_request();

    let response = request.send().promise.await?;
    let data = response.get()?.get_data()?;

    let worker_id = data.get_id()?.to_string()?;
    let generation = data.get_generation();
    let running_vms = data.get_running_vms();

    info!(
        worker_id = %worker_id,
        generation = generation,
        running_vms = running_vms,
        "✓ Got worker status"
    );

    Ok(())
}

pub async fn test_list_vms(addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing Worker.listVms()");

    let worker = connect_to_worker(addr).await?;
    let request = worker.list_vms_request();

    let response = request.send().promise.await?;
    let vms = response.get()?.get_vms()?;

    info!(num_vms = vms.len(), "✓ Listed VMs");
    Ok(())
}
