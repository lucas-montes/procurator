use std::fmt::{Display, Formatter};
use std::net::SocketAddr;

use capnp::message::ReaderOptions;
use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use clap::{Args, Parser, Subcommand};
use futures::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::task::LocalSet;
use tokio_util::compat::TokioAsyncReadCompatExt;

const DEFAULT_ADDR: &str = "0.0.0.0:8080";
const DEFAULT_KERNEL: &str = "result/vmlinux";
const DEFAULT_INITRAMFS: &str = "result/initrd";
const DEFAULT_DISK: &str = "result/rootfs.img";
// Image-specific kernel cmdline file emitted by the `artifacts` derivation
// in the example flake (`nix/examples/python-workload/flake.nix`). Contains
// the NixOS `boot.kernelParams` joined by spaces plus `init=<toplevel>/init`.
// The CLI reads the file verbatim and ships its contents over capnp; the
// worker appends runtime tokens (`procurator.ip=`, `procurator.gw=`) before
// handing the final string to Cloud Hypervisor. The user never sees or edits
// this — Nix owns it.
const DEFAULT_CMDLINE_FILE: &str = "result/cmdline";
const DEFAULT_CONSOLE_MODE: &str = "Off";
const DEFAULT_SERIAL_MODE: &str = "Tty";
const DEFAULT_CPU: u32 = 1;
const DEFAULT_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug)]
enum Error {
    AddressParse(std::net::AddrParseError),
    Rpc(capnp::Error),
    Cmdline(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AddressParse(err) => write!(f, "invalid address: {err}"),
            Self::Rpc(err) => write!(f, "rpc error: {err}"),
            Self::Cmdline(msg) => write!(f, "cmdline: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<capnp::Error> for Error {
    fn from(value: capnp::Error) -> Self {
        Self::Rpc(value)
    }
}

impl From<std::net::AddrParseError> for Error {
    fn from(value: std::net::AddrParseError) -> Self {
        Self::AddressParse(value)
    }
}

#[derive(Debug, Parser)]
#[command(name = "pcr-worker-test", version = "0.1.0")]
#[command(about = "Direct worker RPC test CLI")]
struct Cli {
    #[arg(long, default_value = DEFAULT_ADDR)]
    addr: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Read,
    List,
    Create(CreateArgs),
    Delete(DeleteArgs),
}

#[derive(Debug, Args)]
struct DeleteArgs {
    #[arg(long)]
    id: String,
}

#[derive(Debug, Args)]
struct CreateArgs {
    #[arg(long, default_value = DEFAULT_KERNEL)]
    kernel: String,

    /// Path to the image-specific kernel cmdline file emitted by the
    /// `artifacts` derivation. If omitted, defaults to
    /// `DEFAULT_CMDLINE_FILE`. The file must exist; its contents are sent
    /// over capnp as `payload.cmdline`. The worker appends runtime tokens
    /// (`procurator.ip=`, `procurator.gw=`) before booting the VM.
    #[arg(long)]
    cmdline_file: Option<String>,

    #[arg(long, default_value = DEFAULT_INITRAMFS)]
    initramfs: String,

    #[arg(long, default_value = DEFAULT_DISK)]
    disk: String,

    #[arg(long, default_value_t = DEFAULT_CPU)]
    cpu: u32,

    #[arg(long, default_value_t = DEFAULT_MEMORY_BYTES)]
    memory: u64,

    #[arg(long, default_value = DEFAULT_CONSOLE_MODE)]
    console_mode: String,

    #[arg(long, default_value = DEFAULT_SERIAL_MODE)]
    serial_mode: String,

    #[arg(long)]
    serial_file: Option<String>,

    #[arg(long)]
    tap: Option<String>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();

    if let Err(err) = run().await {
        tracing::error!(?err, "worker test cli failed");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Error> {
    let cli = Cli::parse();
    let addr: SocketAddr = cli.addr.parse()?;
    let local = LocalSet::new();

    local
        .run_until(async move {
            let (worker, rpc_task) = connect(addr).await?;

            match cli.command {
                Command::Read => read(worker).await?,
                Command::List => list(worker).await?,
                Command::Create(args) => create(worker, args).await?,
                Command::Delete(args) => delete(worker, args).await?,
            }

            rpc_task.abort();
            Ok(())
        })
        .await
}

async fn connect(
    addr: SocketAddr,
) -> Result<
    (
        commands::worker_capnp::worker::Client<commands::ch_capnp::vm_config::Owned>,
        tokio::task::JoinHandle<Result<(), capnp::Error>>,
    ),
    Error,
> {
    let stream = TcpStream::connect(addr).await.map_err(capnp::Error::from)?;
    stream.set_nodelay(true).map_err(capnp::Error::from)?;

    let (reader, writer) = stream.compat().split();
    let network = twoparty::VatNetwork::new(
        futures::io::BufReader::new(reader),
        futures::io::BufWriter::new(writer),
        rpc_twoparty_capnp::Side::Client,
        ReaderOptions::default(),
    );

    let mut rpc_system = RpcSystem::new(Box::new(network), None);
    let client: commands::worker_capnp::worker::Client<commands::ch_capnp::vm_config::Owned> =
        rpc_system.bootstrap(rpc_twoparty_capnp::Side::Server);
    let rpc_task = tokio::task::spawn_local(rpc_system);

    Ok((client, rpc_task))
}

fn read_text(reader: capnp::text::Reader<'_>) -> Result<String, Error> {
    reader
        .to_str()
        .map(str::to_owned)
        .map_err(|err| capnp::Error::failed(format!("invalid utf-8 text field: {err}")))
        .map_err(Error::from)
}

/// Read the image-specific base cmdline produced by the flake's `artifacts`
/// derivation. Fails loudly if the file is missing — booting a NixOS VM
/// without the `init=<toplevel>/init` token embedded here dies in stage 1
/// with `/mnt-root//init not found`, and we'd rather surface that at the CLI.
fn read_cmdline_file(explicit: Option<&str>) -> Result<String, Error> {
    let path = explicit.unwrap_or(DEFAULT_CMDLINE_FILE);
    let raw = std::fs::read_to_string(path).map_err(|err| {
        Error::Cmdline(format!(
            "could not read cmdline file '{path}': {err}. \
             Did you run `nix build .#artifacts`?"
        ))
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::Cmdline(format!(
            "cmdline file '{path}' is empty — the flake's artifacts derivation produced nothing"
        )));
    }
    Ok(trimmed.to_owned())
}

async fn read(
    worker: commands::worker_capnp::worker::Client<commands::ch_capnp::vm_config::Owned>,
) -> Result<(), Error> {
    let request = worker.read_request();
    let response = request.send().promise.await?;
    let data = response.get()?.get_data()?;

    println!(
        "Worker status: id={}, healthy={}, generation={}, running_vms={}",
        read_text(data.get_id()?)?,
        data.get_healthy(),
        data.get_generation(),
        data.get_running_vms(),
    );

    Ok(())
}

async fn list(
    worker: commands::worker_capnp::worker::Client<commands::ch_capnp::vm_config::Owned>,
) -> Result<(), Error> {
    let request = worker.list_vms_request();
    let response = request.send().promise.await?;
    let vms = response.get()?.get_vms()?;

    println!("VM count: {}", vms.len());

    for vm in vms.iter() {
        let ip = read_text(vm.get_ip()?)?;
        println!(
            "- id={} worker={} ip={} status={} desired_hash={} observed_hash={} drifted={}",
            read_text(vm.get_id()?)?,
            read_text(vm.get_worker_id()?)?,
            ip,
            read_text(vm.get_status()?)?,
            read_text(vm.get_desired_hash()?)?,
            read_text(vm.get_observed_hash()?)?,
            vm.get_drifted(),
        );
    }

    Ok(())
}

async fn create(
    worker: commands::worker_capnp::worker::Client<commands::ch_capnp::vm_config::Owned>,
    args: CreateArgs,
) -> Result<(), Error> {
    let mut request = worker.create_vm_request();
    let mut spec = request.get().init_spec().init_spec();

    let cmdline = read_cmdline_file(args.cmdline_file.as_deref())?;

    {
        let mut payload = spec.reborrow().init_payload();
        payload.set_kernel(&args.kernel);
        payload.set_cmdline(&cmdline);
        payload.set_initramfs(&args.initramfs);
    }

    {
        let mut cpus = spec.reborrow().init_cpus();
        cpus.set_boot_vcpus(args.cpu);
        cpus.set_max_vcpus(args.cpu);
    }

    {
        let mut memory = spec.reborrow().init_memory();
        memory.set_size(args.memory);
    }

    {
        let mut disks = spec.reborrow().init_disks(1);
        disks.reborrow().get(0).set_path(&args.disk);
    }

    if let Some(tap) = &args.tap {
        let mut net = spec.reborrow().init_net(1);
        net.reborrow().get(0).set_tap(tap);
    }

    {
        let mut console = spec.reborrow().init_console();
        console.set_mode(&args.console_mode);
        console.set_file("");
    }

    {
        let mut serial = spec.reborrow().init_serial();
        serial.set_mode(&args.serial_mode);
        serial.set_file(args.serial_file.as_deref().unwrap_or(""));
    }

    let response = request.send().promise.await?;
    let id = read_text(response.get()?.get_id()?)?;

    println!("Created VM: {id}");
    Ok(())
}

async fn delete(
    worker: commands::worker_capnp::worker::Client<commands::ch_capnp::vm_config::Owned>,
    args: DeleteArgs,
) -> Result<(), Error> {
    let mut request = worker.delete_vm_request();
    request.get().set_id(&args.id);
    request.send().promise.await?;

    println!("Deleted VM: {}", args.id);
    Ok(())
}
