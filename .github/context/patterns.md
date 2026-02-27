# Patterns and Conventions

## Rust Conventions

### General

- Edition 2024
- Workspace-level dependency management (versions in root `Cargo.toml`)
- Workspace-level lints
- `tracing` for all logging (structured, JSON format)
- `tokio` as async runtime everywhere
- Error types are per-module enums, not boxed trait objects (except at binary boundaries)

### Naming

- Crate names: `snake_case` (e.g., `control_plane`, `ci_service`)
- Module files: `snake_case.rs`
- Types: `PascalCase`
- Trait methods: `snake_case`, verb-first (`create`, `delete`, `list`, not `vm_create`)
- Message enums: `CommandPayload::Create`, `CommandPayload::Delete` — verb as variant name
- Response enums: `CommandResponse::Unit`, `CommandResponse::VmList` — noun as variant name
- Error enums: `ChError::Communication`, `ChError::VmNotFound` — noun/adjective as variant

### Crate Structure

Each binary crate follows:
```
src/
├── main.rs      # Minimal: tracing setup, config parsing, call lib::main()
├── lib.rs       # Wiring: creates components, spawns tasks, runs select!
├── server.rs    # RPC interface (if applicable)
├── dto.rs       # Message types, commands, request/response wrappers
└── <domain>/    # Domain-specific modules
```

## Lock-Free Message Passing Pattern

### The Rule

**No `Arc<Mutex<_>>` for shared mutable state.** State is owned by exactly one task. All access goes through message channels.

### The Pattern

```rust
// dto.rs — Define the command enum and response mechanism

pub enum CommandPayload {
    Create(VmSpec),
    Delete(String),
    List,
    GetWorkerStatus,
}

pub enum CommandResponse {
    Unit,
    VmList(Vec<VmInfo>),
    WorkerInfo(WorkerInfo),
}

/// Message sent over the mpsc channel. Contains the command payload
/// and a oneshot reply sender.
pub struct Message {
    data: CommandPayload,
    reply: oneshot::Sender<Result<CommandResponse, VmError>>,
}

/// Thin wrapper around mpsc::Sender<Message> — the caller just passes
/// a CommandPayload and awaits the Result.
#[derive(Clone)]
pub struct CommandSender(mpsc::Sender<Message>);

impl CommandSender {
    pub async fn request(&self, data: CommandPayload) -> Result<CommandResponse, VmError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let msg = Message { data, reply: reply_tx };
        self.0.send(msg).await.map_err(|_| VmError::ManagerDown)?;
        reply_rx.await.map_err(|_| VmError::ManagerDown)?
    }
}
```

```rust
// vm_manager.rs — Single-owner loop, generic over VmmBackend

pub struct VmManager<B: VmmBackend> {
    backend: B,
    vms: HashMap<String, VmHandle<B>>,
}

impl<B: VmmBackend> VmManager<B> {
    pub async fn handle(&mut self, msg: Message) {
        let (cmd, reply) = msg.into_parts();
        let result = match cmd {
            CommandPayload::Create(spec) => self.handle_create(spec).await,
            CommandPayload::Delete(id) => self.handle_delete(id).await,
            CommandPayload::List => self.handle_list().await,
            CommandPayload::GetWorkerStatus => self.handle_get_worker_status().await,
        };
        let _ = reply.send(result);
    }
}
```

```rust
// server.rs — Stateless RPC adapter

#[derive(Clone)]
struct Server {
    sender: CommandSender,
}

// In an RPC handler:
fn list_vms(&mut self, ...) -> Promise<(), capnp::Error> {
    let sender = self.sender.clone();
    Promise::from_future(async move {
        let response = sender.request(CommandPayload::List).await?;
        let CommandResponse::VmList(vms) = response else {
            return Err(capnp::Error::failed("unexpected response".into()));
        };
        // Fill capnp results from vms...
        Ok(())
    })
}
```

### Why This Works

- `VmManager` processes commands **one at a time** — no data races possible
- `Server` clones are trivial (just cloning a `Sender`)
- Backpressure is built in (bounded channel)
- The oneshot reply gives the RPC handler a future to await — capnp's promise system handles the rest

## Cap'n Proto Patterns

### Flat RPC Interfaces

Both the Master and Worker use flat interfaces — all methods are top-level on the bootstrap capability. No nested sub-interfaces or per-object capabilities (yet).

```capnp
interface Worker {
    read @0 () -> (data :Common.WorkerStatus);
    listVms @1 () -> (vms :List(Common.VmStatus));
}
```

**Future consideration:** Per-VM capabilities (object-capability model) were explored in ADR-003 but removed because the supporting operations (getLogs, exec, getConnectionInfo) all require SSH, which is not yet implemented. When SSH support is added, per-VM capabilities may be reintroduced — see ADR-003.

### Private DTO Structs

All message types that cross the channel boundary (VmSpec, VmInfo, WorkerInfo, etc.) have **private fields** with explicit constructors (`new()`) and getters. This enforces construction through validated paths and prevents partial initialization.

```rust
pub struct VmSpec {
    id: String,
    name: String,
    kernel_path: String,
    // ... all private
}

impl VmSpec {
    pub fn new(id: String, name: String, ...) -> Self { ... }
    pub fn id(&self) -> &str { &self.id }
    pub fn kernel_path(&self) -> &str { &self.kernel_path }
}
```

### Zero-Copy Reads

Cap'n Proto messages are read directly from wire format. Avoid:
```rust
// BAD: deserialize capnp → Rust struct → serialize to capnp response
let spec: MyRustStruct = from_capnp(reader)?;
fill_capnp_response(builder, &spec);
```

Prefer:
```rust
// GOOD: read fields directly from capnp reader, write directly to capnp builder
let id = reader.get_id()?;
builder.set_id(id);
```

When interfacing with non-capnp systems (like CH's REST API), conversion is unavoidable. Minimize it by converting at the boundary (VmManager), not in the Server.

### Promise Pipelining

When returning a capability, the caller can immediately call methods on it without waiting for the first RPC to complete:

```rust
// Client side — this is ONE network round trip, not two
let vm = worker.get_vm_request().send().pipeline.get_vm();
let status = vm.read_request().send().promise.await?;
```

The server-side `getVm` impl must return the capability synchronously (or via a resolved promise) for pipelining to work.

## Cloud Hypervisor REST Patterns

### One Process Per VM

CH manages exactly one VM per process. Pattern:

```rust
// VmManager spawns a CH process
let socket_path = format!("/run/procurator/vms/{vm_id}.sock");
let child = Command::new("cloud-hypervisor")
    .arg("--api-socket")
    .arg(&socket_path)
    .spawn()?;

// Create a client for this specific VM
let ch_client = CloudHypervisor::new(&socket_path);
```

### Socket Readiness

After spawning CH, the socket isn't immediately available. Poll with exponential backoff:

```rust
async fn wait_for_socket(path: &Path, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    let mut delay = Duration::from_millis(10);
    while start.elapsed() < timeout {
        if path.exists() {
            return Ok(());
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_millis(500));
    }
    Err(...)
}
```

### Config Construction

CH configs are built from explicit VmSpec paths — no path guessing:

```
VmSpec.kernelPath    = "/nix/store/kernel-abc/bzImage"
VmSpec.diskImagePath = "/nix/store/disk-abc/nixos.raw"
VmSpec.initrdPath    = "/nix/store/initrd-abc/initrd"  (optional)
VmSpec.cmdline       = "console=ttyS0 root=/dev/vda rw"  (optional)
```

The `CloudHypervisorBackend::build_config()` reads these directly from the VmSpec getters.
Kernel, initrd, and disk image are SEPARATE Nix store paths — not subdirectories of a single closure.

## Error Handling

### Per-layer errors

- `ChError` — cloud-hypervisor REST API failures
- `VmError` — VmManager-level errors (VM not found, already exists, nix copy failed)
- `capnp::Error` — RPC-level errors (only at the Server boundary)

### Conversion direction

```
ChError → VmError → capnp::Error
```

Inner errors are wrapped, never leaked across boundaries. The Server converts `VmError` to `capnp::Error::failed(msg)` at the RPC edge.

## Testing Patterns

- **VmManager tests:** create a `mpsc::channel`, send `CommandPayload` messages, assert `CommandResponse` — no network needed
- **Backend testability:** `VmManager<B>` is generic over `VmmBackend`. Tests can provide a mock backend that returns stub `Vmm` clients and `VmmProcess` handles without touching real hypervisors, sockets, or filesystem.
- **Integration tests:** spin up the full Server + Node + VmManager, connect a capnp client
- **CLI test binaries:** `pcr-test` (master), `pcr-worker-test` (worker) for manual end-to-end testing against running servers
- **Unit tests:** in `#[cfg(test)] mod tests` blocks within each module
