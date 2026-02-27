# Architecture

## System Overview

Procurator is a GitOps-driven VM orchestrator. Git commits produce Nix derivations; the control plane schedules them onto workers; workers run them as cloud-hypervisor VMs. No imperative `apply` — the system continuously reconciles to the desired state defined in Git.

```
Git → Nix Eval/Build → Control Plane → Workers → cloud-hypervisor VMs
```

## Component Map

| Crate | Role | Runs as |
|-------|------|---------|
| `control_plane` | Stores desired state, schedules VMs to workers, tracks convergence | Long-lived daemon |
| `worker` | Manages CH processes on a single host, reports observed state | Long-lived daemon per host |
| `cli` | Read-only inspection + manual RPC testing | User-invoked binary |
| `ci_service` | Evaluates Nix, builds closures, publishes to cache, notifies control plane | Triggered by git push |
| `repohub` | Web UI for repo/project management | Long-lived web server |
| `autonix` | Scans repos, generates Nix flakes | Library, called by repohub/ci |
| `commands` | Cap'n Proto schemas + generated code | Build-time library |
| `repo_outils` | Git and Nix utility functions | Library |

## Worker Internal Architecture

The worker is the most complex component. It manages N cloud-hypervisor VM processes on a single host.

### Design Principles

1. **Server is stateless** — it translates RPC calls to messages, nothing more
2. **Single owner of mutable state** — the VmManager owns all VM state in one task, no locks
3. **Message passing over shared memory** — mpsc channels between components
4. **Process isolation** — each VM is a separate CH process; a crash doesn't take down the worker

### Component Diagram

```
                    CLI / Control Plane
                           │
                    Cap'n Proto RPC (TCP)
                           │
                    ┌──────▼──────┐
                    │   Server    │  Stateless RPC adapter
                    │  (cloneable)│  Translates capnp → CommandPayload
                    └──────┬──────┘
                           │ CommandSender (wraps mpsc::Sender<Message>)
                           │
                    ┌──────▼──────┐
                    │    Node<B>  │  Receives messages, drives VmManager
                    │             │  Generic over VmmBackend
                    └──────┬──────┘
                           │
                    ┌──────▼──────┐
                    │ VmManager<B>│  Single tokio task
                    │  (owns all  │  HashMap<VmId, VmHandle>
                    │   VM state) │  Processes commands sequentially
                    └──────┬──────┘
                           │ spawns + talks to (per VM)
              ┌────────────┼────────────┐
              ▼            ▼            ▼
         ┌────────┐   ┌────────┐   ┌────────┐
         │  CH    │   │  CH    │   │  CH    │  cloud-hypervisor processes
         │ proc 1 │   │ proc 2 │   │ proc N │  one per VM
         │ sock 1 │   │ sock 2 │   │ sock N │  REST API over unix socket
         └────────┘   └────────┘   └────────┘
```

### Server

- Implements `worker_capnp::worker::Server`
- Holds only a `CommandSender` (wraps `mpsc::Sender<Message>`) — no VMM reference, no state
- Each RPC method: build a `CommandPayload`, call `sender.request(payload)`, `await` the response
- Freely cloneable by capnp-rpc (Clone just clones the inner Sender)
- Four RPC methods: `read` (worker status), `listVms` (list VMs), `createVm` (takes VmSpec, returns VM ID), `deleteVm` (takes VM ID)

### Node<B: VmmBackend>

- Owns the `Receiver<Message>` and a `VmManager<B>`
- Generic over `VmmBackend` so the entire stack can be tested without real hypervisors
- Main loop: `while let Some(cmd) = self.commands.recv().await { self.manager.handle(cmd).await; }`
- Also holds a `master_addr` for future reconciliation (not used yet)

### VmManager<B: VmmBackend>

- Single-owner of all VM state — `HashMap<String, VmHandle>`
- Generic over `VmmBackend` — production uses `CloudHypervisorBackend`, tests can use a mock
- Receives `Message` (contains `CommandPayload` + oneshot reply) from Node
- Dispatches: `Create`, `Delete`, `List`, `GetWorkerStatus`
- Processes commands **sequentially** — no concurrent mutation, no locks
- For long operations (create VM = spawn CH + REST calls), can spawn a sub-task but updates the HashMap only in the main loop

### VMM Abstraction (three traits in `vmm/interface.rs`)

- **`Vmm`** — per-VM client (one instance = one VM = one socket). Methods: `create`, `boot`, `shutdown`, `delete`, `info`, `pause`, `resume`, `counters`, `ping`. Associated types: `Config`, `Info`, `Error`.
- **`VmmProcess`** — handle to the OS process backing one VM. Methods: `kill`, `cleanup`.
- **`VmmBackend`** — factory that spawns VMM processes and builds configs. Methods: `prepare` (default no-op, override for nix copy), `spawn`, `build_config`. Associated types: `Client: Vmm`, `Process: VmmProcess`.

Production implementations: `CloudHypervisor` (Vmm), `ChProcess` (VmmProcess), `CloudHypervisorBackend` (VmmBackend).

### CloudHypervisor Backend

- `CloudHypervisorBackend` implements `VmmBackend` — factory that spawns CH processes
- `CloudHypervisor` implements `Vmm` — stateless HTTP client to a single CH unix socket
- `ChProcess` implements `VmmProcess` — wraps `tokio::process::Child`
- `CloudHypervisorConfig` holds socket_dir, ch_binary, socket_timeout
- `build_config()` uses explicit paths from VmSpec: `spec.kernel_path()`, `spec.disk_image_path()`, `spec.initrd_path()`, `spec.cmdline()`
- One instance per VM (created by VmManager via the backend when spawning)
- Does NOT track multiple VMs — that's VmManager's job

### Reconciler (future, not needed for CLI testing)

- Periodically pulls desired state from control plane
- Compares with VmManager's actual state (via query messages)
- Sends create/delete commands to VmManager to converge
- This is the "Node" component, renamed for clarity

## Cloud Hypervisor Integration Model

**Key fact: each `cloud-hypervisor` process manages exactly ONE VM.**

There is no multi-VM endpoint. To run 5 VMs, you spawn 5 CH processes, each with its own `--api-socket`.

### VM Lifecycle (managed by VmManager)

```
1. Receive CreateVm command with VmSpec (from capnp)
2. Generate UUIDv7 VM ID (time-sortable, unique)
3. backend.prepare(spec) — ensure Nix closure is local (nix copy, default: no-op)
4. backend.spawn(vm_id) — spawn CH process, wait for socket, return (Client, Process, socket_path)
5. backend.build_config(spec) — resolve store paths → kernel/disk/initrd config
6. client.create(config) — send vm.create REST call
7. client.boot() — send vm.boot REST call
8. Record VmHandle { client, process, spec, socket_path } in HashMap
```

If any step (3–7) fails, no VmHandle is recorded — no orphan state.

### VM Deletion

```
1. Send vm.shutdown REST call (graceful)
2. Wait or timeout
3. Send vm.delete REST call
4. Kill CH process
5. Clean up socket file and TAP device
6. Remove from HashMap
```

### Nix Integration Points

The worker bridges Nix store paths and CH configs:

- **VmSpec** has explicit paths: `kernel_path`, `initrd_path`, `disk_image_path`, `cmdline`
- These are separate Nix store paths (NOT subdirectories of a single closure)
- Worker runs `nix copy --from <binary_cache> <store_path>` to ensure local availability
- The `build_config()` method on `CloudHypervisorBackend` reads these paths directly from the VmSpec
- No path guessing — the CI/build system provides exact store paths via the capnp VmSpec

## Communication Protocol

All inter-service RPC uses **Cap'n Proto**:

- `commands/schema/common.capnp` — Shared data types (VmSpec with 8 fields: toplevel, kernelPath, initrdPath, diskImagePath, cmdline, cpu, memoryMb, networkAllowedDomains; WorkerStatus, etc.)
- `commands/schema/master.capnp` — Control plane interface (publishState, getAssignment, pushData, getClusterStatus, getWorker)
- `commands/schema/worker.capnp` — Worker interface (read, listVms, createVm, deleteVm)
- Generated Rust code lives in `commands/src/lib.rs` via `build.rs`

### Why Cap'n Proto

- **Zero-copy reads** — messages are read directly from wire format, no deserialization step
- **Capabilities** — object references (like a Vm handle) that survive RPC, avoiding ID-per-call lookups
- **Promise pipelining** — call methods on a not-yet-resolved capability, reducing round trips
- **Schema evolution** — add fields without breaking existing clients

## Data Flow: CLI → Worker → CH (current focus)

```
CLI                          Worker Server         Node<B> / VmManager<B>  CH Process
 │                               │                     │                   │
 │── worker.read() ─────────────▶│                     │                   │
 │                               │── GetWorkerStatus ─▶│                   │
 │                               │◀── WorkerInfo ──────│                   │
 │◀── WorkerStatus ─────────────│                     │                   │
 │                               │                     │                   │
 │── worker.listVms() ──────────▶│                     │                   │
 │                               │── List ────────────▶│                   │
 │                               │                     │── (for each vm) ──│
 │                               │                     │   GET /vm.info    │
 │                               │                     │◀── VmInfo ────────│
 │                               │◀── VmList ─────────│                   │
 │◀── List(VmStatus) ───────────│                     │                   │
```

Note: The Server sends `CommandPayload` variants through a `CommandSender` (wraps mpsc).
The Node feeds them to VmManager, which replies via the embedded oneshot channel.

## Nix → Worker Pipeline

How a Nix flake definition becomes a running VM:

```
1. User defines VM in flake.nix using mkVmImage { cpu = 2; memoryMb = 1024; ... }
2. Nix evaluates → produces vmSpec attrset (8 camelCase fields matching capnp VmSpec)
3. mkVmImage also produces vmSpecJson derivation (pkgs.writeText of toJSON vmSpec)
4. CLI reads the JSON: nix build .#my-vm-spec --print-out-paths → /nix/store/...-vm-spec.json
5. CLI deserializes JSON → VmSpec (serde: #[serde(rename_all = "camelCase")])
6. CLI sends createVm RPC with VmSpec fields via capnp
7. Worker receives → VmManager.handle_create():
   a. Generate UUIDv7 VM ID
   b. backend.prepare(&spec)  — ensure Nix closure is in local store
   c. backend.spawn(vm_id)    — spawn CH process
   d. backend.build_config(&spec)  — VmSpec paths → CH config
   e. client.create(config)   — REST: vm.create
   f. client.boot()           — REST: vm.boot
   g. Record VmHandle in HashMap
8. Worker returns VM ID to CLI
```

### Key Design Points

- **VmSpec is the contract** between Nix, CLI, and Worker. 8 fields, camelCase, no optionals.
- **JSON is the serialization format** between Nix and CLI. Cap'n Proto is the wire format between CLI and Worker.
- **`prepare()` is the extension point** for Nix store integration. Default no-op; production backend overrides for `nix copy --from <cache>`.
- **No path guessing** — every path (kernel, initrd, disk) is an explicit field in VmSpec, set by Nix during eval.

## Directory Layout (worker crate)

```
worker/src/
├── main.rs              # Binary entry point, config, tracing setup
├── lib.rs               # Wiring: spawns Server + Node tasks with CloudHypervisorBackend
├── server.rs            # Cap'n Proto RPC implementation (stateless, holds CommandSender)
├── node.rs              # Owns Receiver<Message> + VmManager<B>, processes commands
├── vm_manager.rs        # Single-owner VM state, generic over VmmBackend
├── dto.rs               # CommandPayload/CommandResponse enums, VmSpec/VmInfo/WorkerInfo
│                        #   (private fields, constructors, getters; VmSpec has #[derive(Deserialize)])
├── vm_manager_tests.rs  # Unit tests: create/delete/list/status/failure injection/prepare/JSON deser
└── vmm/
    ├── mod.rs           # Re-exports
    ├── interface.rs     # Three traits: Vmm, VmmProcess, VmmBackend (with prepare())
    ├── cloud_hypervisor.rs  # Production backend: CloudHypervisor, ChProcess, CloudHypervisorBackend
    └── mock.rs          # Test backend: MockBackend, MockVmm, MockProcess (failure injection)
```
