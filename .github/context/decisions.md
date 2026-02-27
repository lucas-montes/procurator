# Architectural Decisions

## ADR-001: Worker wraps cloud-hypervisor rather than exposing CH directly

**Status:** Accepted

**Context:**
Cloud-hypervisor manages exactly one VM per process. There is no multi-VM API — no `list` endpoint, no VM ID parameter on most calls. Each CH process has its own unix socket. A caller wanting to manage N VMs would need to spawn N CH processes and track N sockets themselves.

**Options considered:**

1. **No wrapper — CLI/control-plane talks to CH directly.** Each caller manages CH processes and sockets.
2. **Thin worker wrapper** — Worker spawns CH processes, maps VM IDs to sockets, provides a unified multi-VM API.
3. **Use CH as a library** (link `libcloudh`) — tighter coupling, in-process VM management.

**Decision:** Option 2 — thin wrapper.

**Rationale:**
- CH's one-VM-per-process model requires a multiplexer anyway. The worker is that multiplexer.
- Nix integration (ensuring closures are in the local store, resolving store paths to kernel/disk images) must happen somewhere. It doesn't belong in the CLI or control plane.
- Network setup (TAP devices, bridge attachment) is host-specific and must run on the worker host.
- Process isolation: a CH crash kills one VM, not the worker. Worker can detect and restart.
- Option 3 (library) ties us to CH's Rust internals, which change more often than the REST API. The REST API is stable and versioned.

**Tradeoffs:**
- Extra indirection: CLI → worker RPC → CH REST. But the worker is colocated with CH, so the REST hop is over a unix socket (sub-millisecond).
- Worker is a new daemon to run. Acceptable — it's the whole point of the worker node.

---

## ADR-002: Actor model (message passing) instead of shared-state locks

**Status:** Accepted

**Context:**
The worker's RPC server handles concurrent connections. Cap'n Proto clones the server struct for each connection. If the server holds mutable state (VM table), you need `Arc<Mutex<HashMap<...>>>` or similar. This creates contention, potential deadlocks, and makes the code harder to reason about.

**Options considered:**

1. **`Arc<Mutex<_>>`** — shared state with locks.
2. **`Arc<RwLock<_>>`** — readers don't block each other, but writers still do.
3. **Actor model** — single-owner task processes messages from a channel. No locks.
4. **`dashmap` / lock-free concurrent map** — avoids Mutex but still shared mutable state.

**Decision:** Option 3 — actor model via mpsc channels + oneshot replies.

**Rationale:**
- Zero locks in the critical path. The VmManager owns all VM state and processes commands sequentially in one task.
- The Server becomes trivially cloneable — it only holds a `Sender<VmCommand>`.
- Natural backpressure: bounded channel means the Server blocks if VmManager is overwhelmed.
- Easier to reason about: all state mutations happen in one place, in order.
- Maps perfectly to capnp's `Promise::from_future` pattern — send command, await oneshot, fill response.
- VM operations (spawn process, REST calls) are I/O-bound, not CPU-bound. Sequential processing is fine — we're not doing parallel matrix multiplication.

**Tradeoffs:**
- Throughput ceiling: one command at a time in VmManager. This is fine because VM lifecycle operations are rare (seconds apart) and I/O-bound. If we ever need parallel VM creation, VmManager can spawn sub-tasks for the I/O while keeping HashMap updates serial.
- Channel overhead: ~50ns per send. Irrelevant compared to spawning a VM (seconds).
- Slightly more boilerplate than direct method calls (enum variants, oneshot channels).

---

## ADR-003: Cap'n Proto capabilities for per-VM identity

**Status:** Deferred — implementation removed, concept preserved for future SSH integration

**Context:**
The `Worker.getVm(vmId)` RPC was designed to return a `Vm` capability — a per-VM object with methods like `read()`, `getLogs()`, `exec()`, and `getConnectionInfo()`.

**Original Decision:** Option 3 — dedicated `VmCapability` struct with vm_id + Sender, implementing `worker::vm::Server`.

**Current State:**
The Vm sub-interface was removed because ALL its interesting methods (getLogs, exec, getConnectionInfo) require SSH into the guest VM, which is not yet implemented. Keeping the interface as a collection of TODO stubs added complexity without value. The removal simplifies the worker schema to a flat interface:
- `Worker.read @0` — worker status
- `Worker.listVms @1` — list VMs

**What was removed:**
- `worker.capnp`: entire `Vm` sub-interface and `getVm @2` method
- `master.capnp`: `getVm @5` method
- `common.capnp`: `VmLogs`, `ExecOutput`, `ConnectionInfo` structs
- `server.rs`: `VmCapability` struct
- `dto.rs`: `GetInfo`, `GetLogs`, `Exec`, `GetConnectionInfo` command variants

**When to re-introduce:**
When SSH support is implemented (see Questions #6 in todo.md), the per-VM capability pattern should return. The VmCapability struct would hold a vm_id + CommandSender and scope operations to a single VM. Promise pipelining (`worker.getVm("foo").read()` → one round trip) is a key benefit worth preserving.

**Rationale for deferral:**
- Dead code with TODO bodies adds noise and gives a false sense of progress
- The flat interface is simpler to test and reason about
- Adding the Vm capability back is additive — it doesn't require a rewrite

---

## ADR-004: One cloud-hypervisor process per VM with REST API

**Status:** Accepted

**Context:**
Cloud-hypervisor can be run as a standalone process with `--api-socket` or embedded as a library. The worker needs to manage VM lifecycles.

**Decision:** Spawn one `cloud-hypervisor` process per VM, communicate via REST over unix socket.

**Rationale:**
- **Process isolation:** a VM crash (or CH bug) kills only that CH process, not the worker or other VMs.
- **Clean cleanup:** `kill` the process to guarantee the VM is gone. No leaked state.
- **Stable API:** CH's REST API is versioned and documented (OpenAPI spec in `cloud-hypervisor.yaml`). Rust internals change between releases.
- **Observable:** each CH process can be inspected independently (`/api/v1/vm.info`).
- **No unsafe / FFI:** pure HTTP client, no linking to C or CH's Rust crate.

**Tradeoffs:**
- Per-VM process overhead: ~10-20MB RSS per idle CH process. Acceptable for tens of VMs. If we ever need thousands, revisit.
- Socket file management: worker must clean up stale sockets on restart. Solvable with a `/run/procurator/vms/` directory that gets cleaned on startup.
- Startup latency: spawning a process + waiting for socket + REST calls adds ~200-500ms vs in-process. Acceptable for VM lifecycle operations.

---

## ADR-005: Vmm trait stays as per-VM interface, VmManager is the multi-VM layer

**Status:** Accepted

**Context:**
The existing `Vmm` trait (`interface.rs`) defines `create`, `delete`, `info`, `list`. But CH's one-process-per-VM model means `list` doesn't map to a single CH socket. There are two possible levels of abstraction.

**Options considered:**

1. **Make Vmm trait multi-VM** — `create(id, config)`, `delete(id)`, `info(id)`, `list()`. The impl manages the process table internally.
2. **Keep Vmm trait per-VM** — one Vmm instance = one VM. A separate VmManager handles the collection.

**Decision:** Option 2 — keep Vmm as per-VM, add VmManager as the collection layer.

**Rationale:**
- The Vmm trait cleanly maps to CH's actual API: one socket, one VM.
- If we ever support a different hypervisor that IS multi-VM (e.g., libvirt, Firecracker with jailer), we can implement Vmm differently — but the VmManager layer stays the same.
- Separation of concerns: Vmm = "how to talk to one hypervisor", VmManager = "how to manage a fleet".

**Tradeoffs:**
- `list()` on the Vmm trait is awkward (CH can't list). Consider removing it from the trait and making it a VmManager-only operation. The trait becomes: `create`, `delete`, `info`, `shutdown`, `boot`.

---

## ADR-006: Nix closures as the VM artifact format

**Status:** Accepted

**Context:**
VMs need kernel images, root filesystem images, and configuration. These could be Docker images, raw disk images built by Packer, or Nix closures.

**Decision:** Nix closures are the sole VM artifact format.

**Rationale:**
- **Reproducibility:** same commit → same closure hash → same VM image. Always.
- **Deduplication:** Nix store deduplicates shared dependencies across VMs. 10 VMs with the same base system share the same store paths.
- **Binary cache:** `nix copy` distributes pre-built closures. Workers don't need to build anything — they just pull from cache.
- **Drift detection:** compare `VmSpec.contentHash` with running hash. If different → replace. Deterministic because Nix hashes are content-addressed.
- **Everything is in the closure:** kernel, initramfs, disk image, boot config. No external dependencies at runtime.
- **Aligns with the GitOps model:** Git → Nix eval → closure → deploy. No imperative steps.

**Tradeoffs:**
- Learning curve: Nix is complex. But this project IS a Nix orchestrator — the team must know Nix.
- Build time: NixOS images take minutes to build. Mitigated by binary cache — builds happen once in CI.
- Image size: minimal NixOS images are 500-700MB. Larger than Alpine containers but include everything (kernel, init, userspace). No image pull at runtime — it's already in the store.

---

## ADR-007: Start with CLI-driven testing, add reconciliation loop later

**Status:** Accepted

**Context:**
The full system has a reconciliation loop (control plane → worker → CH). But testing the full loop requires the control plane, CI pipeline, and binary cache to be operational.

**Decision:** Phase the implementation:
1. **Now:** CLI → Worker → VmManager → CH. Manual VM creation/deletion for testing.
2. **Next:** Add Reconciler that pulls from control plane and drives VmManager.
3. **Later:** Full GitOps loop with CI triggering reconciliation.

**Rationale:**
- Get the VmManager and CH integration working first. This is the hardest part.
- CLI testing validates the capnp schema, message passing, and CH process management without needing other services.
- The Reconciler is an additional message source — it sends the same `VmCommand` messages as the Server. Adding it later is additive, not a rewrite.

**Tradeoffs:**
- CLI-driven testing allows imperative commands (`create vm X`), which contradicts the GitOps model. That's fine for testing — the Reconciler will be the production entry point.

---

## ADR-008: No `apply` command — Git is the only write interface

**Status:** Accepted (for production; relaxed for testing per ADR-007)

**Context:**
Traditional orchestrators have `kubectl apply` or `terraform apply`. This creates drift between what's in Git and what's running.

**Decision:** In production, the only way to change cluster state is to push to Git. The CI pipeline evaluates, builds, and publishes. The control plane distributes. Workers reconcile.

**Rationale:**
- Single source of truth: Git.
- Audit trail: every change is a commit with author and timestamp.
- Rollback: `git revert && git push`. No special rollback machinery.
- No partial applies: either the entire generation succeeds evaluation and build, or nothing changes.

**Tradeoffs:**
- Slower feedback loop than `kubectl apply`. Mitigated by fast CI and binary cache.
- Can't do ad-hoc debugging by pushing a quick config change. SSH into the VM instead, or use the worker's `exec` capability.

---

## ADR-009: Generic VmmBackend for testable VM management

**Status:** Accepted

**Context:**
The VmManager directly instantiated CloudHypervisor clients and managed CH processes. This made it impossible to unit-test VmManager without spawning real cloud-hypervisor processes, creating unix sockets, and having a working hypervisor binary on the test machine.

**Options considered:**

1. **Mock the HTTP layer** — intercept unix socket calls to CH. Complex, tests the HTTP client more than the manager.
2. **Trait on VmManager methods** — make VmManager itself a trait. Over-abstracting — VmManager is the concrete orchestrator.
3. **Three-trait abstraction** — `Vmm` (per-VM client), `VmmProcess` (process handle), `VmmBackend` (factory). VmManager is generic over `VmmBackend`.

**Decision:** Option 3 — three-trait abstraction with `VmManager<B: VmmBackend>`.

**Rationale:**
- Maps cleanly to CH's architecture: one process per VM (VmmProcess), one REST client per VM (Vmm), one factory (VmmBackend)
- Tests can provide a mock backend that returns stub clients and processes without touching filesystem, network, or hypervisors
- The generics propagate to Node<B> but stop at the Server (which only holds a CommandSender, not the backend)
- Production wiring in lib.rs: `CloudHypervisorBackend::new(config)` — the concrete type is chosen at the binary boundary
- If we ever support a different hypervisor (e.g., Firecracker, QEMU), it's a new backend implementation, not a VmManager rewrite

**Trait summary:**
- `Vmm`: create, boot, shutdown, delete, info, pause, resume, counters, ping. Associated types: Config, Info, Error.
- `VmmProcess`: kill, cleanup.
- `VmmBackend`: prepare(spec), spawn(vm_id) → (Client, Process, PathBuf), build_config(spec) → Config.

**Tradeoffs:**
- More type parameters — `VmManager<B>`, `Node<B>`, `VmHandle<B>`. Contained to the worker crate internals.
- `impl Trait` in trait methods (RPITIT) requires Rust edition 2024. We're already on edition 2024.

---

## ADR-010: prepare() on VmmBackend instead of separate ArtifactResolver trait

**Status:** Accepted

**Context:**
Before a VM can be spawned, the worker must ensure that the Nix closure (kernel, disk image, initrd) is available in the local store. In the typical deployment flow: Nix builds on CI → pushes to a binary cache → worker needs to `nix copy --from <cache> <store_path>` before it can use the paths.

The question: where does this "ensure artifacts are local" logic live?

**Options considered:**

1. **Separate `VmArtifactResolver` trait** — a second generic on VmManager: `VmManager<B: VmmBackend, R: VmArtifactResolver>`. The resolver trait has one method: `resolve(spec) → Result<ResolvedSpec>`. Clean separation of concerns, but doubles the generics.
2. **Method on VmmBackend** — add `prepare(&self, spec: &VmSpec) -> Result<(), VmError>` to the existing VmmBackend trait with a default no-op. Production backend overrides to run `nix copy`. Tests get the no-op for free.
3. **Standalone function** — a free function called by VmManager before spawn. Not mockable, not overridable per-backend.

**Decision:** Option 2 — `prepare()` on `VmmBackend` with default no-op.

**Rationale:**
- **Avoids premature abstraction:** We have exactly one backend (CloudHypervisor). A second generic parameter adds type complexity for a single implementation. If we ever need to mix-and-match resolvers and backends independently, we can extract the trait then.
- **No double generics:** `VmManager<B>` stays single-generic. No `VmManager<B, R>`, no extra PhantomData, no dual trait bounds propagating through Node and lib.rs.
- **Default no-op keeps tests trivial:** MockBackend inherits the default no-op (or overrides it for failure injection testing), without needing a separate MockResolver.
- **Semantically correct:** "prepare the environment for this spec" IS a backend concern — different backends may need different preparation (nix copy for CH, OCI pull for containers, nothing for local dev).
- **Easily overridable:** CloudHypervisorBackend can override `prepare()` later to run `nix copy --from <cache> <store_paths>` without changing VmManager or any other code.
- **Call order is clear:** VmManager's `handle_create` calls `prepare()` → `spawn()` → `build_config()` → `create()` → `boot()`. The prepare step runs before any process is spawned, so failures are clean (no orphan processes).

**Tradeoffs:**
- Couples artifact resolution to the backend. If we need the same resolution logic for multiple backends, we'd duplicate it. Acceptable for now — we have one backend.
- The default no-op could silently hide a missing override. Mitigated by integration tests that actually exercise the Nix path.
