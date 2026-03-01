//! Testing CLI for manually testing Procurator RPC interfaces during development.
//!
//! Covers all Worker RPC methods defined in worker.capnp:
//! - read: fetch worker status
//! - list-vms: list all managed VMs
//! - create-vm: create a VM from a spec (JSON file or individual flags)
//! - delete-vm: destroy a VM by ID

use clap::{Args, Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;

mod worker_client;

#[derive(Debug, Parser)]
#[command(name = "pcr-test", version = "0.1.0")]
#[command(about = "Development test client for Procurator Worker server")]
struct Cli {
    /// Worker server address
    #[arg(short, long, default_value = "127.0.0.1:6000")]
    addr: SocketAddr,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Fetch worker status (Worker.read)
    Read,

    /// List all VMs (Worker.listVms)
    ListVms,

    /// Create a VM from a spec (Worker.createVm)
    CreateVm(CreateVmArgs),

    /// Delete a VM by ID (Worker.deleteVm)
    DeleteVm(DeleteVmArgs),
}

#[derive(Debug, Args)]
struct CreateVmArgs {
    /// Path to a VM spec JSON file (output of `nix build .#vmSpecJson`)
    #[arg(long, conflicts_with_all = ["kernel_path", "disk_image_path"])]
    spec_file: Option<PathBuf>,

    /// /nix/store path to system toplevel
    #[arg(long, required_unless_present = "spec_file")]
    toplevel: Option<String>,

    /// /nix/store path to kernel (bzImage)
    #[arg(long, required_unless_present = "spec_file")]
    kernel_path: Option<String>,

    /// /nix/store path to initrd
    #[arg(long, required_unless_present = "spec_file")]
    initrd_path: Option<String>,

    /// /nix/store path to root disk image
    #[arg(long, required_unless_present = "spec_file")]
    disk_image_path: Option<String>,

    /// Kernel command line
    #[arg(long, default_value = "console=ttyS0 root=/dev/vda rw init=/sbin/init")]
    cmdline: String,

    /// Number of vCPUs
    #[arg(long, default_value = "1")]
    cpu: u32,

    /// RAM in megabytes
    #[arg(long, default_value = "512")]
    memory_mb: u32,

    /// Allowed network domains (can be repeated)
    #[arg(long)]
    allowed_domain: Vec<String>,
}

#[derive(Debug, Args)]
struct DeleteVmArgs {
    /// VM ID to delete
    id: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let client = worker_client::connect(cli.addr).await?;

            match cli.command {
                Commands::Read => worker_client::read(&client).await?,
                Commands::ListVms => worker_client::list_vms(&client).await?,
                Commands::CreateVm(args) => {
                    let spec = args.resolve()?;
                    worker_client::create_vm(&client, spec).await?;
                }
                Commands::DeleteVm(args) => {
                    worker_client::delete_vm(&client, &args.id).await?;
                }
            }

            Ok(())
        })
        .await
}

/// JSON-deserialisable VM spec matching the Nix vmSpecJson output.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmSpecJson {
    pub toplevel: String,
    pub kernel_path: String,
    pub initrd_path: String,
    pub disk_image_path: String,
    pub cmdline: String,
    pub cpu: u32,
    pub memory_mb: u32,
    #[serde(default)]
    pub network_allowed_domains: Vec<String>,
}

impl CreateVmArgs {
    fn resolve(self) -> Result<VmSpecJson, Box<dyn std::error::Error>> {
        if let Some(path) = self.spec_file {
            let contents = std::fs::read_to_string(&path)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
            let spec: VmSpecJson = serde_json::from_str(&contents)
                .map_err(|e| format!("invalid JSON in {}: {e}", path.display()))?;
            Ok(spec)
        } else {
            Ok(VmSpecJson {
                toplevel: self.toplevel.unwrap_or_default(),
                kernel_path: self.kernel_path.ok_or("--kernel-path required")?,
                initrd_path: self.initrd_path.ok_or("--initrd-path required")?,
                disk_image_path: self.disk_image_path.ok_or("--disk-image-path required")?,
                cmdline: self.cmdline,
                cpu: self.cpu,
                memory_mb: self.memory_mb,
                network_allowed_domains: self.allowed_domain,
            })
        }
    }
}
