# Control Plane Node State

In-memory desired-state store behind the `Master` RPC surface. Implemented
slice: `NodeState` + channel loop in `control_plane/src/node.rs`, message
types in `control_plane/src/dto.rs`, explicit placement in
`control_plane/src/scheduler.rs`, storage in `control_plane/src/store.rs`,
inventory in `control_plane/src/registry.rs`, file loading in
`control_plane/src/config.rs`.

Worker poll + heartbeat run against this surface (`worker/src/pull.rs`,
`worker/src/heartbeat.rs`); cross-crate e2e lives in `worker/src/e2e.rs`.
Not yet wired: persistence (restart loses all generations).

## Spec identity + payloads (plan T09)

`Common.VmSpec` carries additive `id` + `targetWorker` (wire-compatible;
absent reads as `""`). Publish maps each spec to `DesiredEntry{key, target,
spec-bytes}`: blank ids and unparseable targets fail loudly through the
`Result` union, undeclared targets fail in Node — then the store retains the
opaque bytes per (worker, key), replaced wholesale per publish (automatic
pruning). `getAssignment` rebuilds each `desiredVm` from retained bytes
(validated key + requesting worker for identity, bytes for the backend spec;
response lists are branded `any_pointer` by generated code, so the backend
crosses via `set_as`). Same-key-different-target is last-write-wins,
documented; no scheduler change.

## RPC translation (`server.rs`, T04 narrowed, T09 payloads)

Thin delegation from `Master` handlers to the node channel
(`NodeMessenger::request`).

| Method | Wire → Node | Reply mapping |
|---|---|---|
| `publishState` | commit/generation/intentHash + specs (id/target validated, bytes retained) | `Published` → `ok`; `NodeError` → `err` text |
| `getAssignment` | `workerId` (validated) + `lastSeen` | `Assignment` → generation + rebuilt `desiredVms` |
| `pushData` | `RunningVm.id` → keys (blank ids rejected); healthy = true-on-receipt (liveness only); metrics debug-logged, zero-filled downstream | `Observed` → `ok` |
| `getClusterStatus` | — | snapshot → generation/commit/percent + worker rows (zeroed resources/metrics, empty `vms`) |
| `getWorker` | validated id → node lookup → `WorkerDirectory` (see below) | unknown/unavailable → Cap'n Proto failure |

Content rejections answer through the `Result` union; only broken framing
or a lost node channel becomes a `Promise` failure.

## Worker capability directory (`directory.rs`, T04)

Swappable seam for `Worker` RPC clients: `WorkerDirectory` trait,
`NoWorkerDirectory` (always unavailable — pull mode never dials workers),
`Server::with_directory` injection for a future push transport. Deliberately
non-generic and `Send`/`Sync`-free: the generated capability setter is fixed
to `Client<any_pointer>` (erasure at this boundary is sound), and capability
clients are thread-local like the `spawn_local` loop.

## Event → reply

| Event | Reply | Notes |
|---|---|---|
| `PublishState{commit, generation, intent_hash, entries}` | `Published(generation)` or `NodeError` | Full desired-state replace per generation; entries carry spec bytes |
| `GetAssignment{worker_id, last_seen}` | `Assignment{generation, vms[key+spec]}` | Empty slice (never error) when unknown, unpublished, or up-to-date |
| `PushData{report}` | `Observed` | Always succeeds; undeclared workers ignored |
| `GetClusterStatus` | `Cluster(ClusterSnapshot)` | Read-only snapshot |
| `GetWorker{worker_id}` | `Worker(Option<View>)` | `None` when undeclared |

Every loop iteration replies exactly once (`NodeMessenger::request`).
Transport failures (`Send`, `ReplyClosed`) are distinct from node
rejections (`NodeError`), so RPC mapping (T04) can tell them apart.

## Validation rules (`NodeState::publish`)

- Blank commit → `EmptyCommit`; nothing stored.
- `generation <= active` → `StaleGeneration{proposed, active}`; nothing stored.
- Entry target outside declared inventory → `Schedule(UnknownWorker)`; nothing stored.
- Generations start at 1 (0 is never newer per store poll semantics).

## Explicit placement (`explicit_map`)

Pure `desired × declared inventory → per-worker slices`. v1 pins every VM
to a target worker; no bin-packing, labels, or resource fit. Unknown targets
rejected (typos never silently drop VMs). Keys sorted per worker for
deterministic polls.

## Convergence (`ClusterSnapshot`)

- Per worker `up_to_date`: observed generation == active generation **and**
  reported running keys equal desired keys (order-insensitive).
- `convergence_percent`: up-to-date / declared workers, 0–100; 100 with no
  declared workers (vacuous). `is_converged` matches.
- Heartbeats carry `ObservedReport{worker_id, observed_generation,
  running: [VmKey], healthy}`; `running.len()` feeds `ObservedState`.

## Inventory seeding

`Config::declared_workers()` (from Nix `workers`, T02) → `Node::new`
via `control_plane::main` ← crate binary `main.rs`. Fail-fast: unreadable,
unparsable, or duplicate/blank-id inventory exits the process. Peer
discovery from the `role` envelope stays dormant (no HA).

## See also

- [../plans/control-plane-pull-assignments.md](../plans/control-plane-pull-assignments.md) — implementation plan (complete)
- [../worker/pull-reconcile.md](../worker/pull-reconcile.md) — worker pull + heartbeat
- [../architecture.md](../architecture.md) — system architecture
- [../context-map.md](../context-map.md) — navigation
