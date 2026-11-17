# Console (as built)

Read-only web UI that shows the cluster live, built **only from the logs the
services already write**. It performs no actions and makes no RPC calls;
services don't know it exists. Committed as `6817a90`. Plan:
`plans/control-plane-web-console.md`. Log convention:
`specs/log-events.md`. User-facing doc: `console/README.md`.

## Decisions (agreed with the user)
- Read-only: "the console does nothing, only shows information".
- Data comes only from parsing each service's own `tracing` JSON logs. No
  shared telemetry crate, no logs pushed to the control plane.
- Real-time only, no history. A short bootstrap read-back at startup rebuilds
  current state from the periodic state lines.
- Log source is a per-service config knob: `file` (manual runs, one machine),
  `journal_gateway` (systemd-journal-gatewayd per host), `journal_local`
  (`journalctl -f`).
- Frontend as simple as possible: embedded HTML/CSS/one vanilla JS file.
- No tests on the JSON shape of the traces. Drift shows up as unparsed lines
  on the Sources page; bump `log.schema` on breaking changes.
- journal-gatewayd has no auth: accepted for now (trusted network).

## Service side (what writes the lines)
- **Logging setup:** `control_plane/src/logging.rs`, `worker/src/logging.rs`.
  Config `log` section: `format` `json|pretty`, optional
  `file {path, rotation never|daily}`. Stdout is synchronous, file writes are
  non-blocking (`tracing-appender`, guard kept in `main`).
- **Message lines:** `control_plane/src/msglog.rs`, `worker/src/msglog.rs`.
  A `Msg` drop guard logs one line per RPC with from/to/method/outcome/
  latency. The callee logs; the caller logs only when the callee isn't ours
  (Cloud Hypervisor) or on transport/connect failure. Servers keep the peer
  address per connection so `client:<addr>` endpoints are real.
- **Control plane state:** `Node::log_state()` on a ticker
  (`state_log_interval_millis`, default 10000) plus a `generation` change line
  on publish.
- **Worker state:** `heartbeat::run_state_log_loop` (heartbeat interval),
  independent of the control-plane connection. Host metrics from
  `worker/src/metrics.rs` (`/proc`), per-VM stats from `Handle::stats()`
  (CH: uptime, pid CPU/RSS, `vm.counters` for net/disk). Change lines:
  `vm_created`, `vm_start_failed`, `vm_deleted`, `reconciled`.
- **CH calls:** `vmm::ch` client `call(method, action, body)` logs `vmm`
  message lines.

## Console crate (`console/`)
- `src/config.rs`: `http_addr` (default `127.0.0.1:8095`),
  `bootstrap_entries` (2000), `stale_after_millis` (15000), `sources`
  `[{name, service: control_plane|worker:<id>, kind}]`. `service` resolves the
  `self` endpoint.
- `src/sources/`: `file.rs` (tail, rotation check), `gateway.rs`
  (`Range: entries=:-N:` bootstrap, then resume from the last `__CURSOR`,
  backoff up to 30 s), `journal_local.rs`, `journal_entry.rs` (journal export
  JSON → line).
- `src/parse.rs`: strict convention parse, lenient fallback for plain lines.
- `src/model.rs`: single owner task. Machines/VMs from state lines, missing/
  orphan VM states, CPU % from `cpu_ms` deltas, staleness, VM pruning per tick.
  Messages and logs are live only (older than `LIVE_SLACK_MS` = 2 s are not
  fanned out, so the bootstrap doesn't replay old traffic).
- `src/http.rs`: `/` and page routes serve one HTML shell; `/assets/app.js`,
  `/assets/app.css`; `GET /api/model` snapshot; **one** SSE `/api/stream`
  with `state`, `message`, `log`, `lagged` events.
- `assets/app.js`: client routing, SVG map with pulses per message kind,
  machine/unit/VM/sources pages, client-side filtering.

## Nix
- `nix/modules/console.nix` (`services.procurator.console`): `controlPlane`/
  `workers` names from `cluster.vms` → `journal_gateway` sources,
  `local.*` → `journal_local`, `extraSources` raw. DynamicUser in
  `systemd-journal` group.
- `nix/modules/journal-gateway.nix` (`services.procurator.journalGateway`:
  `enable`, `port` 19531, `openFirewall`).
- Package `console` in `nix/flake/packages.nix`; check `nixos-console-fast`
  (`nix/tests/console-fast.nix`).

## Verified
- `cargo test`: control_plane 29, worker 60, console 12. Clippy `-D warnings`
  clean for the three crates.
- Pure-eval checks: `console-fast`, `control-plane-fast`,
  `worker-service-fast`, `services-system-eval` (fixed: it now declares a
  cache, it was broken by the cache rewrite).
- Manual e2e: real CP + worker logging to files; real journal-gatewayd
  (incl. disconnect + cursor resume); Chromium via puppeteer (map pulses,
  feeds update, no JS errors, repeated reloads don't stall).
- Not done: building `console` through Nix/naersk, `nix flake check`.

## Lessons
- Three SSE streams per page exhausted Chrome's six HTTP/1.1 connections per
  host after two reloads, stalling the page. Hence the single merged stream.

## Known gaps / next
- **Inside-VM data (plan T15):** deferred; to discuss with the worker's VM
  logs (ideas: failed/restarting units, guest service logs, boot health via a
  guest journal-gatewayd).
- **Deployment units** aren't published, so every VM is "ungrouped". Units
  (a "master" flake grouping service flakes deployed together) are a
  separate plan.
- CH `vmm` lines carry the factory-minted VM id, not the desired key: the map
  draws them on the machine's VMM anchor, the VM page doesn't list them.
- Worker heartbeat wire metrics stay zero; real metrics only go to state
  lines.
- No auth/TLS on the console (loopback by default) or on journal-gatewayd.
- **Unrelated bug found, not fixed:** after a control-plane restart a worker
  can delete all its VMs. `InMemoryAssignmentStore::assignment_for`
  (`control_plane/src/store.rs`) answers `last_seen >= active` with an empty
  VM list, and the worker (`worker/src/pull.rs`) reconciles whenever the
  generation differs. Needs its own fix plan.
