//! Manual test CLI for debugging Procurator RPC operations
use clap::{Parser, Subcommand};
use std::net::SocketAddr;

use capnp_rpc::{rpc_twoparty_capnp, twoparty, RpcSystem};
use futures::AsyncReadExt;

#[derive(Parser, Debug)]
#[command(name = "pcr-test")]
#[command(about = "Manual test CLI for debugging Procurator RPC", long_about = None)]
struct Args {
    /// Master server address
    #[arg(short, long, default_value = "127.0.0.1:5000")]
    addr: SocketAddr,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Get cluster status
    Status,
    /// Get worker info
    Worker {
        /// Worker ID
        worker_id: String,
    },
    /// Get VM info
    Vm {
        /// VM ID
        vm_id: String,
    },
    /// Get VM logs
    Logs {
        /// VM ID
        vm_id: String,
        /// Follow logs
        #[arg(short, long)]
        follow: bool,
        /// Number of lines to tail
        #[arg(short = 'n', long, default_value = "10")]
        tail: u32,
    },
    /// Get assignment for worker
    Assignment {
        /// Worker ID
        worker_id: String,
        /// Last seen generation
        #[arg(short, long, default_value = "0")]
        generation: u64,
    },
    /// Push observed state from worker
    Push {
        /// Worker ID
        worker_id: String,
        /// Observed generation
        #[arg(short, long, default_value = "1")]
        generation: u64,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive(tracing::Level::INFO.into()))
        .init();

    let args = Args::parse();

    let local = tokio::task::LocalSet::new();
    local.run_until(async move {
        // Connect to master - bootstraps directly to MasterControl
        let stream = tokio::net::TcpStream::connect(args.addr).await?;
        stream.set_nodelay(true)?;

        let (reader, writer) = tokio_util::compat::TokioAsyncReadCompatExt::compat(stream).split();
        let rpc_network = Box::new(twoparty::VatNetwork::new(
            futures::io::BufReader::new(reader),
            futures::io::BufWriter::new(writer),
            rpc_twoparty_capnp::Side::Client,
            Default::default(),
        ));

        let mut rpc_system = RpcSystem::new(rpc_network, None);
        let master: commands::master_control::Client =
            rpc_system.bootstrap(rpc_twoparty_capnp::Side::Server);

        tokio::task::spawn_local(rpc_system);

        match args.command {
            Commands::Status => {
                let status_req = master.get_cluster_status_request();
                let resp = status_req.send().promise.await?;
                let status = resp.get()?.get_status()?;

                println!("Cluster Status:");
                println!("  Generation: {}", status.get_active_generation());
                println!("  Commit: {}", status.get_active_commit()?.to_str()?);
                println!("  Convergence: {}%", status.get_convergence_percent());
                println!("  Workers: {}", status.get_workers()?.len());
                println!("  VMs: {}", status.get_vms()?.len());
            }

            Commands::Worker { worker_id } => {
                let mut worker_req = master.get_worker_request();
                worker_req.get().set_worker_id(&worker_id);
                let resp = worker_req.send().promise.await?;
                let worker = resp.get()?.get_worker()?;

                let read_req = worker.read_request();
                let read_resp = read_req.send().promise.await?;
                let data = read_resp.get()?.get_data()?;

                println!("Worker: {}", worker_id);
                println!("  ID: {}", data.get_id()?.to_str()?);
                println!("  Healthy: {}", data.get_healthy());
                println!("  Generation: {}", data.get_generation());
                println!("  Running VMs: {}", data.get_running_vms());
            }

            Commands::Vm { vm_id } => {
                let mut vm_req = master.get_vm_request();
                vm_req.get().set_vm_id(&vm_id);
                let resp = vm_req.send().promise.await?;
                let vm = resp.get()?.get_vm()?;

                let read_req = vm.read_request();
                let read_resp = read_req.send().promise.await?;
                let data = read_resp.get()?.get_data()?;

                println!("VM: {}", vm_id);
                println!("  ID: {}", data.get_id()?.to_str()?);
                println!("  Worker: {}", data.get_worker_id()?.to_str()?);
                println!("  Status: {}", data.get_status()?.to_str()?);
                println!("  Drifted: {}", data.get_drifted());
                println!("  Desired Hash: {}", data.get_desired_hash()?.to_str()?);
                println!("  Observed Hash: {}", data.get_observed_hash()?.to_str()?);
            }

            Commands::Logs { vm_id, follow, tail } => {
                let mut vm_req = master.get_vm_request();
                vm_req.get().set_vm_id(&vm_id);
                let resp = vm_req.send().promise.await?;
                let vm = resp.get()?.get_vm()?;

                let mut logs_req = vm.get_logs_request();
                logs_req.get().set_follow(follow);
                logs_req.get().set_tail_lines(tail);
                let logs_resp = logs_req.send().promise.await?;
                let logs = logs_resp.get()?.get_logs()?;

                println!("{}", logs.get_logs()?.to_str()?);
                if logs.get_truncated() {
                    println!("\n[truncated]");
                }
            }

            Commands::Assignment { worker_id, generation } => {
                // For worker commands, we need to connect to WorkerControl interface
                // In production, this would connect to the worker's address, not master
                // For testing, we can connect to master which also implements WorkerControl

                // Reconnect with WorkerControl bootstrap
                let stream2 = tokio::net::TcpStream::connect(args.addr).await?;
                stream2.set_nodelay(true)?;
                let (reader2, writer2) = tokio_util::compat::TokioAsyncReadCompatExt::compat(stream2).split();
                let rpc_network2 = Box::new(twoparty::VatNetwork::new(
                    futures::io::BufReader::new(reader2),
                    futures::io::BufWriter::new(writer2),
                    rpc_twoparty_capnp::Side::Client,
                    Default::default(),
                ));
                let mut rpc_system2 = RpcSystem::new(rpc_network2, None);
                let worker_control: commands::worker_control::Client =
                    rpc_system2.bootstrap(rpc_twoparty_capnp::Side::Server);
                tokio::task::spawn_local(rpc_system2);

                let mut assign_req = worker_control.get_assignment_request();
                assign_req.get().set_worker_id(&worker_id);
                assign_req.get().set_last_seen_generation(generation);

                let resp = assign_req.send().promise.await?;
                let result = resp.get()?.get_result()?;

                match result.which()? {
                    commands::result::Which::Ok(assignment) => {
                        let assignment = assignment?;
                        println!("Assignment for {}", worker_id);
                        println!("  Generation: {}", assignment.get_generation());
                        println!("  VMs: {}", assignment.get_desired_vms()?.len());

                        for (i, vm) in assignment.get_desired_vms()?.iter().enumerate() {
                            println!("\n  VM {}:", i + 1);
                            println!("    ID: {}", vm.get_id()?.to_str()?);
                            println!("    Name: {}", vm.get_name()?.to_str()?);
                            println!("    Hash: {}", vm.get_content_hash()?.to_str()?);
                        }
                    }
                    commands::result::Which::Err(err) => {
                        eprintln!("Error: {}", err?.to_str()?);
                        std::process::exit(1);
                    }
                }
            }

            Commands::Push { worker_id, generation } => {
                // For worker commands, reconnect with WorkerControl bootstrap
                let stream2 = tokio::net::TcpStream::connect(args.addr).await?;
                stream2.set_nodelay(true)?;
                let (reader2, writer2) = tokio_util::compat::TokioAsyncReadCompatExt::compat(stream2).split();
                let rpc_network2 = Box::new(twoparty::VatNetwork::new(
                    futures::io::BufReader::new(reader2),
                    futures::io::BufWriter::new(writer2),
                    rpc_twoparty_capnp::Side::Client,
                    Default::default(),
                ));
                let mut rpc_system2 = RpcSystem::new(rpc_network2, None);
                let worker_control: commands::worker_control::Client =
                    rpc_system2.bootstrap(rpc_twoparty_capnp::Side::Server);
                tokio::task::spawn_local(rpc_system2);

                let mut push_req = worker_control.push_observed_state_request();
                {
                    let mut params = push_req.get();
                    params.set_worker_id(&worker_id);
                    params.set_observed_generation(generation);

                    // Sample data
                    {
                        let mut running_vms = params.reborrow().init_running_vms(1);
                        let mut vm0 = running_vms.reborrow().get(0);
                        vm0.set_id("test-vm");
                        vm0.set_content_hash("abc123");
                        vm0.set_status("running");
                        vm0.set_uptime(3600);
                    }

                    {
                        let mut metrics = params.reborrow().init_metrics();
                        metrics.set_available_cpu(8.0);
                        metrics.set_available_memory(16_000_000_000);
                        metrics.set_disk_usage(50_000_000_000);
                        metrics.set_uptime(86400);
                    }
                }

                let resp = push_req.send().promise.await?;
                let result = resp.get()?.get_result()?;

                match result.which()? {
                    commands::result::Which::Ok(_) => {
                        println!("✓ Successfully pushed observed state for {}", worker_id);
                    }
                    commands::result::Which::Err(err) => {
                        eprintln!("Error: {}", err?.to_str()?);
                        std::process::exit(1);
                    }
                }
            }
        }

        Ok::<(), Box<dyn std::error::Error>>(())
    }).await?;

    Ok(())
}
