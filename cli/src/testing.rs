//! Client functions for testing Master server interfaces

use capnp_rpc::{rpc_twoparty_capnp, twoparty, RpcSystem};
use commands::{common_capnp, master_capnp};
use futures::AsyncReadExt;
use std::net::SocketAddr;
use tracing::{info, error};

/// Connect to the Master server and return a Master client
async fn connect_to_master(
    addr: SocketAddr,
) -> Result<master_capnp::master::Client, Box<dyn std::error::Error>> {
    info!(addr = %addr, "Connecting to Master server");

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
    let client: master_capnp::master::Client = rpc_system.bootstrap(rpc_twoparty_capnp::Side::Server);

    tokio::task::spawn_local(rpc_system);

    info!("Connected successfully");
    Ok(client)
}

async fn connect_to<T: capnp::capability::FromClientHook>(
    addr: SocketAddr,
) -> Result<T, Box<dyn std::error::Error>> {
    info!(addr = %addr, "Connecting to Master server");

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
    let client: T = rpc_system.bootstrap(rpc_twoparty_capnp::Side::Server);

    tokio::task::spawn_local(rpc_system);

    info!("Connected successfully");
    Ok(client)
}

/// Test Master.publishState() interface
pub async fn test_publish(
    addr: SocketAddr,
    commit: String,
    generation: u64,
    intent_hash: String,
    num_vms: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(
        commit = %commit,
        generation = generation,
        intent_hash = %intent_hash,
        num_vms = num_vms,
        "Testing Master.State.publish()"
    );

    let master = connect_to_master(addr).await?;

    // Build VmSpecs
    let mut request = master.publish_state_request();

    request.get().set_commit(&commit);
    request.get().set_generation(generation);
    request.get().set_intent_hash(&intent_hash);

    // Create sample VM specs
    {
        let mut vm_specs = request.get().init_vm_specs(num_vms as u32);
        for i in 0..num_vms {
            let mut vm = vm_specs.reborrow().get(i as u32);
            vm.set_toplevel(&format!("/nix/store/fakehash-nixos-system-vm-{}", i));
            vm.set_kernel_path(&format!("/nix/store/kernel-{}/bzImage", i));
            vm.set_initrd_path(&format!("/nix/store/initrd-{}/initrd", i));
            vm.set_disk_image_path(&format!("/nix/store/disk-{}/nixos.raw", i));
            vm.set_cmdline("console=ttyS0 root=/dev/vda rw");
            vm.set_cpu(1);
            vm.set_memory_mb(1024);
            let _domains = vm.init_network_allowed_domains(0);
        }
    }

    info!("Sending publish request...");
    let response = request.send().promise.await?;

    let result = response.get()?.get_result()?;
    match result.which()? {
        common_capnp::result::Ok(ok) => {
            let _empty = ok?;
            info!("✓ Publish succeeded");
        }
        common_capnp::result::Err(err) => {
            let err_msg = err?.to_string()?;
            error!(error = %err_msg, "✗ Publish failed");
            return Err(format!("Publish error: {}", err_msg).into());
        }
    }

    Ok(())
}

/// Test Master.getAssignment() interface
pub async fn test_get_assignment(
    addr: SocketAddr,
    worker_id: String,
    last_seen_generation: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(
        worker_id = %worker_id,
        last_seen_generation = last_seen_generation,
        "Testing Master.getAssignment()"
    );

    let master = connect_to_master(addr).await?;

    let mut request = master.get_assignment_request();

    request.get().set_worker_id(&worker_id);
    request.get().set_last_seen_generation(last_seen_generation);

    info!("Sending getAssignment request...");
    let response = request.send().promise.await?;

    let result = response.get()?.get_result()?;
    match result.which()? {
        common_capnp::result::Ok(ok) => {
            let assignment = ok?;
            let generation = assignment.get_generation();
            let vms = assignment.get_desired_vms()?;
            info!(
                generation = generation,
                num_vms = vms.len(),
                "✓ Got assignment"
            );
        }
        common_capnp::result::Err(err) => {
            let err_msg = err?.to_string()?;
            error!(error = %err_msg, "✗ getAssignment failed");
            return Err(format!("getAssignment error: {}", err_msg).into());
        }
    }

    Ok(())
}

/// Test Master.pushData() interface
pub async fn test_push_data(
    addr: SocketAddr,
    worker_id: String,
    observed_generation: u64,
    num_vms: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(
        worker_id = %worker_id,
        observed_generation = observed_generation,
        num_vms = num_vms,
        "Testing Master.pushData()"
    );

    let master = connect_to_master(addr).await?;

    let mut request = master.push_data_request();

    request.get().set_worker_id(&worker_id);
    request.get().set_observed_generation(observed_generation);

    // Create sample running VMs
    {
        let mut running_vms = request.get().init_running_vms(num_vms as u32);
        for i in 0..num_vms {
            let mut vm = running_vms.reborrow().get(i as u32);
            vm.set_id(&format!("vm-{}", i));
            vm.set_content_hash(&format!("sha256:fakehash{}", i));
            vm.set_status("running");
            vm.set_uptime(3600); // 1 hour

            let mut metrics = vm.init_metrics();
            metrics.set_cpu_usage(0.5);
            metrics.set_memory_usage(512 * 1024 * 1024);
            metrics.set_network_rx_bytes(1024 * 1024);
            metrics.set_network_tx_bytes(512 * 1024);
        }
    }

    // Add worker metrics
    {
        let mut metrics = request.get().init_metrics();
        metrics.set_available_cpu(4.0);
        metrics.set_available_memory(8 * 1024 * 1024 * 1024);
        metrics.set_disk_usage(100 * 1024 * 1024 * 1024);
        metrics.set_uptime(86400); // 1 day
    }

    info!("Sending pushData request...");
    let response = request.send().promise.await?;

    let result = response.get()?.get_result()?;
    match result.which()? {
        common_capnp::result::Ok(ok) => {
            let _empty = ok?;
            info!("✓ pushData succeeded");
        }
        common_capnp::result::Err(err) => {
            let err_msg = err?.to_string()?;
            error!(error = %err_msg, "✗ pushData failed");
            return Err(format!("pushData error: {}", err_msg).into());
        }
    }

    Ok(())
}

/// Test Master.getClusterStatus() interface
pub async fn test_get_cluster_status(
    addr: SocketAddr,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing Master.getClusterStatus()");

    let master = connect_to_master(addr).await?;

    let request = master.get_cluster_status_request();

    info!("Sending getClusterStatus request...");
    let response = request.send().promise.await?;

    let status = response.get()?.get_status()?;
    let generation = status.get_active_generation();
    let commit = status.get_active_commit()?.to_string()?;
    let convergence = status.get_convergence_percent();
    let workers = status.get_workers()?;
    let vms = status.get_vms()?;

    info!(
        generation = generation,
        commit = %commit,
        convergence_percent = convergence,
        num_workers = workers.len(),
        num_vms = vms.len(),
        "✓ Got cluster status"
    );

    Ok(())
}

/// Test Master.getWorker() interface
pub async fn test_get_worker(
    addr: SocketAddr,
    worker_id: String,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(worker_id = %worker_id, "Testing Master.getWorker()");

    let master = connect_to_master(addr).await?;

    let mut request = master.get_worker_request();

    request.get().set_worker_id(&worker_id);

    info!("Sending getWorker request...");
    match request.send().promise.await {
        Ok(response) => {
            let _worker = response.get()?.get_worker()?;
            info!("✓ Got worker capability");

            // Could test calling methods on the worker capability here
            // For example: worker.read_request().send().promise.await?
        }
        Err(e) => {
            error!(error = ?e, "✗ getWorker failed (expected - not implemented)");
            return Err(e.into());
        }
    }

    Ok(())
}
