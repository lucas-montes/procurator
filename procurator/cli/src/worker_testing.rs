//! Client functions for testing Worker server interfaces

use capnp_rpc::{rpc_twoparty_capnp, twoparty, RpcSystem};
use commands::worker_capnp;
use futures::AsyncReadExt;
use std::net::SocketAddr;
use tracing::{info, error};

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

async fn get_vm_client(
    addr: SocketAddr,
    vm_id: String,
) -> Result<worker_capnp::worker::vm::Client, Box<dyn std::error::Error>> {
    let worker = connect_to_worker(addr).await?;
    let mut request = worker.get_vm_request();
    request.get().set_vm_id(&vm_id);

    let response = request.send().promise.await?;
    let vm = response.get()?.get_vm()?;
    Ok(vm)
}

pub async fn test_vm_read(
    addr: SocketAddr,
    vm_id: String,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(vm_id = %vm_id, "Testing Worker.Vm.read()");

    let vm = get_vm_client(addr, vm_id).await?;
    let request = vm.read_request();
    let response = request.send().promise.await?;

    let data = response.get()?.get_data()?;
    let id = data.get_id()?.to_string()?;
    let status = data.get_status()?.to_string()?;

    info!(vm_id = %id, status = %status, "✓ Got VM status");
    Ok(())
}

pub async fn test_vm_logs(
    addr: SocketAddr,
    vm_id: String,
    follow: bool,
    tail_lines: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(vm_id = %vm_id, follow = follow, tail_lines = tail_lines, "Testing Worker.Vm.getLogs()");

    let vm = get_vm_client(addr, vm_id).await?;
    let mut request = vm.get_logs_request();
    request.get().set_follow(follow);
    request.get().set_tail_lines(tail_lines);

    let response = request.send().promise.await?;
    let logs = response.get()?.get_logs()?;

    let content = logs.get_logs()?.to_string()?;
    let truncated = logs.get_truncated();

    info!(truncated = truncated, "✓ Got VM logs");
    if !content.is_empty() {
        println!("{content}");
    }

    Ok(())
}

pub async fn test_vm_exec(
    addr: SocketAddr,
    vm_id: String,
    command: String,
    args: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(vm_id = %vm_id, command = %command, "Testing Worker.Vm.exec()");

    let vm = get_vm_client(addr, vm_id).await?;
    let mut request = vm.exec_request();
    request.get().set_command(&command);

    let mut args_list = request.get().init_args(args.len() as u32);
    for (idx, arg) in args.iter().enumerate() {
        args_list.set(idx as u32, arg);
    }

    let response = request.send().promise.await?;
    let output = response.get()?.get_output()?;

    let stdout = output.get_stdout()?.to_string()?;
    let stderr = output.get_stderr()?.to_string()?;
    let exit_code = output.get_exit_code();

    info!(exit_code = exit_code, "✓ Exec completed");
    if !stdout.is_empty() {
        println!("stdout:\n{stdout}");
    }
    if !stderr.is_empty() {
        eprintln!("stderr:\n{stderr}");
    }

    Ok(())
}

pub async fn test_vm_connection_info(
    addr: SocketAddr,
    vm_id: String,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(vm_id = %vm_id, "Testing Worker.Vm.getConnectionInfo()");

    let vm = get_vm_client(addr, vm_id).await?;
    let request = vm.get_connection_info_request();
    let response = request.send().promise.await?;

    let info_data = response.get()?.get_info()?;
    let host = info_data.get_worker_host()?.to_string()?;
    let ssh_port = info_data.get_ssh_port();
    let user = info_data.get_username()?.to_string()?;

    info!(host = %host, ssh_port = ssh_port, user = %user, "✓ Got connection info");

    Ok(())
}
