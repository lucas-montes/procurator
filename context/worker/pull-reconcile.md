# Worker Pull + Reconcile

Worker-side pull loop against the control-plane `Master` surface.
Implemented slices: poll + reconcile (`worker/src/pull.rs`, plan T05) and
heartbeat reporting (`worker/src/heartbeat.rs`, plan T06); config in
`worker/src/config.rs`, task wiring in `worker/src/lib.rs`.
Cross-crate proof in `worker/src/e2e.rs` (plan T07): real control plane +
real pull pieces over loopback RPC with fake execution — publish →
poll/create → heartbeat → converged status → removal → converged again.

## Loop (`run_poll_loop` + `poll_loop`)

- Dials `master_addr`, polls `getAssignment(workerId, lastSeenGeneration)`.
- Tracks last applied generation; same-generation polls are silent no-ops.
- Transport loss returns a reconnect signal (redial with backoff); server
  rejections and reconcile failures back off in place (capped exponential,
  30 s ceiling) without killing the worker.
- Runs on the crate `LocalSet` next to the RPC server; capability clients
  are thread-local, so the client trait carries no `Send`/`Sync`.

## Reconciler rules (`Reconciler::reconcile`)

- Diffs desired keys against `Registry<Reader>`; creates missing, deletes
  extras (sorted), never touches up-to-date VMs (no spurious restarts).
- Creates decode spec bytes iteration-locally, build via
  `Factory::create_vm`, then re-key the factory-minted id to the desired key
  (`CreateCommand::into_parts`); emits `Command::Create/Delete` on the
  existing supervisor channel.
- Outcome reports created/deleted ids in command order.

## Identity (plan T09)

Each desired `VmSpec` carries an `id` used as the reconcile key; blank ids
are skipped with a warning (fail-safe: never manage what cannot be
identified). Response specs are branded `any_pointer` by generated code;
bytes keep their wire brand and are reinterpreted as the backend struct at
materialize time (identical struct layout, same deployment backend). The
[`Reconciler`] itself stays key-opaque. (Pre-T09 index-derived keys are
gone.)

## Config (`worker/src/config.rs`)

- `worker_id: String` — required, blank rejected at startup (fail-fast).
- `master_addr: SocketAddr` — required (already emitted by
  `nix/lib/worker.nix`).
- `poll_interval_millis` — default 2000; `heartbeat_interval_millis` —
  default 5000; `nix/lib/worker.nix` also emits `worker_id` (`worker-1`
  dev default) so generated configs parse. `service.nix` options land in T07.

## Heartbeat (`heartbeat.rs`, T06)

- `build_report`: registry → `{worker_id, observed_generation, running[],
  healthy}`; per-VM status `running`/`failed` via `health()`, worker health
  is the conjunction, ids sorted for stable payloads.
- Mapping mirrors `listVms`/`read` exactly: `contentHash ""`, uptimes `0`,
  zeroed metrics/resources (all untracked, like `read()`).
- Observed generation comes from the shared `AppliedGeneration` cell the
  poll loop writes (0 until the first poll lands — liveness first).
- Own redialing connection (`run_heartbeat_loop`); `Rejected` backs off in
  place, `Transport` returns for redial, dial failures back off. Errors never
  escape — a missed control plane cannot crash the poller.
- `PushClient` trait (mocked in tests) with the real impl on
  `RpcPullClient`; thread-local like the poll client.

## Tests

- Shared fakes in `worker/src/test_support.rs` (fake backend with
  content-ignoring spec conversion, temp sqlite databases).
- Mock-`PullClient` script tests (create/delete/no-op/backoff/transport),
  plus a loopback stub-`Master` test proving the real `RpcPullClient`
  path including the index-key bridge.
- Mock-`PushClient` script tests (retry/backoff/transport) plus a loopback
  stub-`Master` test capturing `pushData` wire shape (ids, statuses,
  generation) and the rejection mapping.

## See also

- [../control-plane/node-state.md](../control-plane/node-state.md) — control-plane assignment semantics
- [../plans/control-plane-pull-assignments.md](../plans/control-plane-pull-assignments.md) — implementation plan (complete)
- [public-proxy.md](./public-proxy.md) — worker public proxy
- [../context-map.md](../context-map.md) — navigation
