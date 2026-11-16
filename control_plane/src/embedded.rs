//! The control plane inside another process: the monolith runs the node and
//! its Cap'n Proto server on a thread of their own and deploys through a
//! [`Publisher`].
//! Why: the server is single-threaded (`LocalSet`, `!Send` capabilities)
//! while the monolith is not; the node's channel is all they share.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use tokio::sync::mpsc::channel;

use crate::config::Config;
use crate::dto::{NodeEvent, NodeMessenger, NodeReply};
use crate::node::Node;
use crate::registry::WorkerId;
use crate::server::Server;
use crate::store::{DesiredEntry, VmKey};

/// Messages queued between the publisher, the server and the node.
const CHANNEL: usize = 100;
/// The worker overwrites the disk path and serial; these only satisfy CH.
const DISK_TYPE: &str = "raw";
const CONSOLE_OFF: &str = "Off";

/// Why a deployment was not published.
#[derive(Debug)]
pub struct PublishError(String);

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for PublishError {}

/// What a worker boots for one VM: Cloud Hypervisor direct kernel boot from
/// store paths the worker has locally.
#[derive(Debug, Clone, Copy)]
pub struct VmImage<'a> {
    kernel: &'a str,
    initramfs: &'a str,
    cmdline: &'a str,
    disk: &'a str,
    cpus: u32,
    memory_bytes: u64,
}

impl<'a> VmImage<'a> {
    /// A one-CPU, 1 GiB VM booting `kernel` and `initramfs` with
    /// `cmdline`, whose root disk is a copy of the raw image `disk`.
    #[must_use]
    pub fn new(kernel: &'a str, initramfs: &'a str, cmdline: &'a str, disk: &'a str) -> Self {
        Self {
            kernel,
            initramfs,
            cmdline,
            disk,
            cpus: 1,
            memory_bytes: 1 << 30,
        }
    }

    /// The same image with `cpus` vCPUs and `memory_bytes` of memory.
    #[must_use]
    pub fn with_resources(mut self, cpus: u32, memory_bytes: u64) -> Self {
        self.cpus = cpus;
        self.memory_bytes = memory_bytes;
        self
    }
}

/// One VM of a deployment, pinned to a worker.
#[derive(Debug)]
pub struct DesiredVm(DesiredEntry);

impl DesiredVm {
    /// VM `id` on `worker`, booting `image`.
    ///
    /// # Errors
    ///
    /// When `id` or `worker` is blank, or the spec cannot be encoded.
    pub fn new(id: &str, worker: &str, image: &VmImage<'_>) -> Result<Self, PublishError> {
        let key = VmKey::new(id).map_err(|e| PublishError(e.to_string()))?;
        let target = WorkerId::new(worker).map_err(|e| PublishError(e.to_string()))?;
        let spec = encode(id, worker, image).map_err(|e| PublishError(format!("{id}: {e}")))?;
        Ok(Self(DesiredEntry::new(key, target).with_spec(spec)))
    }
}

/// The bytes the worker decodes: a `VmSpec(ch.VmConfig)` message, as
/// `publishState` stores them.
fn encode(id: &str, worker: &str, image: &VmImage<'_>) -> Result<Vec<u8>, capnp::Error> {
    let mut message = capnp::message::Builder::new_default();
    {
        let mut root = message.init_root::<commands::common_capnp::vm_spec::Builder<
            '_,
            commands::ch_capnp::vm_config::Owned,
        >>();
        root.set_id(id);
        root.set_target_worker(worker);
        let mut spec = root.init_spec();
        let mut payload = spec.reborrow().init_payload();
        payload.set_kernel(image.kernel);
        payload.set_cmdline(image.cmdline);
        payload.set_initramfs(image.initramfs);
        let mut cpus = spec.reborrow().init_cpus();
        cpus.set_boot_vcpus(image.cpus);
        cpus.set_max_vcpus(image.cpus);
        spec.reborrow().init_memory().set_size(image.memory_bytes);
        let mut disk = spec.reborrow().init_disks(1).get(0);
        disk.set_path(image.disk);
        disk.set_image_type(DISK_TYPE);
        spec.reborrow().init_console().set_mode(CONSOLE_OFF);
        spec.reborrow().init_serial().set_mode(CONSOLE_OFF);
    }
    let mut bytes = Vec::new();
    capnp::serialize::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

/// Deploys to an embedded control plane.
#[derive(Debug, Clone)]
pub struct Publisher {
    messenger: NodeMessenger,
}

impl Publisher {
    /// Make `vms` the whole desired state, built from `commit`, as the next
    /// generation. `intent` identifies the deployment: when it is the
    /// active one, nothing changes. Returns the active generation.
    ///
    /// # Errors
    ///
    /// When the node is gone or rejects the deployment (a worker outside
    /// its inventory, a blank commit).
    pub fn deploy(
        &self,
        commit: &str,
        intent: &str,
        vms: Vec<DesiredVm>,
    ) -> impl Future<Output = Result<u64, PublishError>> + Send + '_ {
        let event = NodeEvent::Deploy {
            commit: String::from(commit),
            intent_hash: String::from(intent),
            entries: vms.into_iter().map(|vm| vm.0).collect(),
        };
        async move {
            let reply = self
                .messenger
                .request(event)
                .await
                .map_err(|e| PublishError(e.to_string()))?;
            match reply {
                Ok(NodeReply::Published(generation)) => Ok(generation.value()),
                Ok(other) => Err(PublishError(format!("unexpected reply {other:?}"))),
                Err(rejected) => Err(PublishError(rejected.to_string())),
            }
        }
    }
}

/// Start the control plane of `config` on a thread of its own, serving
/// workers on `config.addr`, and return a publisher for it. It runs until
/// the process exits.
///
/// # Errors
///
/// When the inventory is invalid, the address cannot be bound, or the
/// thread cannot start.
pub fn start(config: &Config) -> Result<Publisher, PublishError> {
    let declared = config
        .declared_workers()
        .map_err(|e| PublishError(format!("worker inventory: {e}")))?;
    let addr = config.addr;
    let listener = std::net::TcpListener::bind(addr)
        .and_then(|listener| listener.set_nonblocking(true).map(|()| listener))
        .map_err(|e| PublishError(format!("bind {addr}: {e}")))?;
    let interval = Duration::from_millis(config.state_log_interval_millis.get());
    let (sender, receiver) = channel(CHANNEL);
    let messenger = NodeMessenger::from(sender);
    let for_server = messenger.clone();
    let node = Node::new(receiver, Vec::new(), declared).with_state_log_interval(interval);
    std::thread::Builder::new()
        .name(String::from("control-plane"))
        .spawn(move || {
            // Why: the server is `!Send`, so it is made on its own thread.
            let server: Server<capnp::any_pointer::Owned> = Server::new(for_server);
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::error!(%error, "control plane runtime");
                    return;
                }
            };
            let local = tokio::task::LocalSet::new();
            local.block_on(&runtime, async move {
                tokio::task::spawn(node.run());
                let outcome = match tokio::net::TcpListener::from_std(listener) {
                    Ok(listener) => server.serve_on(listener).await,
                    Err(error) => Err(error.into()),
                };
                if let Err(error) = outcome {
                    tracing::error!(%error, "control plane server stopped");
                }
            });
        })
        .map_err(|e| PublishError(format!("control plane thread: {e}")))?;
    tracing::info!(%addr, "control plane started");
    Ok(Publisher { messenger })
}

#[cfg(test)]
mod tests {
    use super::{DesiredVm, VmImage, encode, start};
    use crate::config::{Config, WorkerEntry};

    fn image() -> VmImage<'static> {
        VmImage::new(
            "/nix/store/k/vmlinux",
            "/nix/store/i/initrd",
            "console=ttyS0 init=/nix/store/t/init",
            "/nix/store/d/nixos.img",
        )
        .with_resources(2, 512 << 20)
    }

    #[test]
    fn specs_decode_as_the_worker_reads_them() {
        let bytes = encode("web", "w1", &image()).unwrap();
        let message = capnp::serialize::read_message(
            &mut bytes.as_slice(),
            capnp::message::ReaderOptions::default(),
        )
        .unwrap();
        let root = message
            .get_root::<commands::common_capnp::vm_spec::Reader<'_, commands::ch_capnp::vm_config::Owned>>()
            .unwrap();
        assert_eq!(root.get_id().unwrap().to_str().unwrap(), "web");
        assert_eq!(root.get_target_worker().unwrap().to_str().unwrap(), "w1");
        let spec = root.get_spec().unwrap();
        let payload = spec.get_payload().unwrap();
        assert_eq!(
            payload.get_kernel().unwrap().to_str().unwrap(),
            "/nix/store/k/vmlinux"
        );
        assert_eq!(
            payload.get_initramfs().unwrap().to_str().unwrap(),
            "/nix/store/i/initrd"
        );
        assert_eq!(spec.get_cpus().unwrap().get_boot_vcpus(), 2);
        assert_eq!(spec.get_memory().unwrap().get_size(), 512 << 20);
        let disk = spec.get_disks().unwrap().get(0);
        assert_eq!(
            disk.get_path().unwrap().to_str().unwrap(),
            "/nix/store/d/nixos.img"
        );
        assert_eq!(disk.get_image_type().unwrap().to_str().unwrap(), "raw");
    }

    #[tokio::test]
    async fn deployments_reach_the_embedded_node() {
        let config = Config {
            hostname: String::from("cp"),
            addr: "127.0.0.1:0".parse().unwrap(),
            workers: vec![WorkerEntry {
                id: String::from("w1"),
                addr: None,
                labels: Vec::new(),
            }],
            state_log_interval_millis: std::num::NonZeroU64::new(60_000).unwrap(),
        };
        let publisher = start(&config).unwrap();
        let vms = || vec![DesiredVm::new("web", "w1", &image()).unwrap()];
        assert_eq!(
            publisher
                .deploy("abc", "/nix/store/x-1", vms())
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            publisher
                .deploy("abc", "/nix/store/x-1", vms())
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            publisher
                .deploy("def", "/nix/store/x-2", vms())
                .await
                .unwrap(),
            2
        );
        let ghost = vec![DesiredVm::new("web", "ghost", &image()).unwrap()];
        let error = publisher
            .deploy("ghi", "/nix/store/x-3", ghost)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("ghost"), "{error}");
        assert!(DesiredVm::new(" ", "w1", &image()).is_err());
    }
}
