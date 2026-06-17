# Plan: Service orchestrator lifecycle improvements

**Plan ID:** orchestrator-lifecycle
**Created:** 2026-06-18

## Change summary

Fix the service orchestrator (`cli/pcr/stack/service.rs`) so that:

1. **Kill errors are surfaced** — `let _ = svc.kill().await;` silently drops kill failures; errors must be logged and propagated where possible.
2. **Orphaned children are prevented** — if the supervisor is killed (including `SIGKILL`), child processes remain running as zombies; children must be tied to the supervisor's lifetime via process groups and/or parent-death signals.
3. **Unexpected service death is detected** — if a service process exits on its own (crash, OOM, signal), the supervisor currently never notices; the supervisor must monitor child lifetimes and react.

## Success criteria

- `./kill_all` and `./remove` never silently drop a kill/wait error — each is at minimum logged via `tracing::warn!` or `tracing::error!`.
- After a `SIGKILL` of the supervisor process, all child processes are reaped within a few seconds (Linux). On other platforms, `kill_on_drop` covers the normal-drop and panic paths.
- Unexpected service exit is detected within one poll cycle (≤1 s) and logged. A restart policy is applied (config-driven or always-restart for long-running services).
- All existing `pcr` stack unit tests continue to pass.
- Manual smoke test with `mock/` stack: `server` crash (kill -9) triggers restart, client reconnects without manual intervention.

## Constraints and non-goals

- **In scope only:** Changes to `cli/pcr/stack/service.rs` and `cli/pcr/stack/config.rs` (expose the already-present `restart` field). No new crates or files.
- **Out of scope:** Full healthcheck system (separate plan `healthcheck.md`). Service readiness probes. Metrics collection. Persistent restart counters or circuit breakers (simple backoff only). Non-Linux process management beyond `kill_on_drop`.
- **Pragmatism over perfection:** `prctl(PR_SET_PDEATHSIG)` is Linux-only and guarded by `#[cfg(target_os = "linux")]`. Other platforms get `kill_on_drop` only, which handles the 99 % case (drop after panic or supervisor restart).
- **Existing `restart` field:** The `Service` struct already has `restart: Option<String>` but it is unused and not exposed via a getter. T03 will expose it and honour `"always"` (restart on any exit) as a recognised value.

## Task stack

### T01: Surface kill/wait errors instead of swallowing them

- [ ] T01: `Surface kill/wait errors instead of swallowing them` (status:todo)
  - Task ID: T01
  - Goal: Replace every `let _ = svc.kill().await;` with code that logs the error if the kill or wait fails.
  - Boundaries (in/out of scope):
    - In: `RunningManifest::kill_all()`, `RunningManifest::remove()`.
    - Out: The `Service::kill()` method itself already returns `Result` — no change needed there.
  - Done when:
    - `kill_all()` logs each per-service kill/wait error via `tracing::error!` (or `warn!`) instead of discarding it.
    - `remove()` does the same.
    - Existing test suite passes.
  - Verification notes (commands or checks):
    - `cargo test -p cli`
    - Code review: confirm no remaining `let _ = svc.kill()` or `let _ = .kill()` in `service.rs`.

### T02: Tie child lifetime to supervisor lifetime

- [ ] T02: `Tie child lifetime to supervisor lifetime` (status:todo)
  - Task ID: T02
  - Goal: Ensure child processes are cleaned up when the supervisor exits, whether by normal shutdown, panic, or `SIGKILL`.
  - Boundaries (in/out of scope):
    - In: `tokio::process::Command::kill_on_drop(true)` on every spawned service. On Linux only: a `pre_exec` hook that calls `prctl(PR_SET_PDEATHSIG, SIGTERM)` so children receive SIGTERM if the parent dies.
    - Out: Changes to the `RunningManifest::kill_all` path (already signals children by T01). Alternative OS mechanisms outside Linux.
  - Done when:
    - `kill_on_drop(true)` is set in `Service::start()` before `.spawn()`.
    - Linux `#[cfg]` block calls `prctl(PR_SET_PDEATHSIG, SIGTERM)` via `libc` or raw `unsafe` syscall in a `pre_exec` closure.
    - Manual test: start mock stack, `kill -9 $supervisor_pid`, observe that all child pids are gone within 2 s (Linux).
    - Existing tests still pass.
  - Verification notes (commands or checks):
    - `cargo test -p cli`
    - Manual: `pkill -9 pcr` → `ps aux | grep -E 'nc|python3.*server|python3.*client'` returns empty.
    - Note: The `mock/` stack now uses Python server (see sibling change) which is a single stable process, making the test cleaner.

### T03: Detect and react to unexpected service death

- [ ] T03: `Detect and react to unexpected service death` (status:todo)
  - Task ID: T03
  - Goal: Monitor running services and restart them when they exit unexpectedly, driven by the `restart` config field.
  - Boundaries (in/out of scope):
    - In:
      - Spawn a per-service monitoring tokio task that calls `handle.wait()` and sends a notification through a channel to the supervisor.
      - The supervisor loop gains a new `select!` branch for service-exit events.
      - On unexpected exit: if `restart == "always"` (or implicit `true` for non-oneShot services), respawn the service with a 1 s backoff delay (no exponential backoff for now).
      - Expose the existing `Service.restart` field via a getter so the supervisor can read it. Recognised values: `"always"` (restart on any exit). `null` / absent → do not restart (current behaviour).
    - Out: Exponential backoff / circuit breaker. Restart-limit tracking. Pre-stop / post-start hooks. Healthcheck probes (separate plan).
  - Done when:
    - A long-running service that exits (e.g. `kill -9` its PID) is automatically restarted by the supervisor within 2 s.
    - A oneShot service that exits is NOT restarted (unchanged behaviour).
    - The `restart` field in `mock/flake.nix` can be set to `"always"` and the supervisor honours it.
    - All existing tests pass.
  - Verification notes (commands or checks):
    - `cargo test -p cli`
    - Manual smoke test with mock stack:
      1. Start mock stack → `server` is running.
      2. `kill -9 $(pgrep -f 'python3.*server.py')` → supervisor logs unexpected exit and restarts server.
      3. Client reconnects automatically.
    - Code review: supervisor event loop handles `ServiceExited(name, status)` without panicking.

### T04: Validation and context sync

- [ ] T04: `Validation and context sync` (status:todo)
  - Task ID: T04
  - Goal: Full build, test suite, manual smoke test, lint/format, and context sync.
  - Boundaries (in/out of scope):
    - In: `cargo test`, `cargo fmt --check`, `cargo clippy -p cli`, manual smoke test with mock stack, sync `context/` docs.
    - Out: Integration tests that require Nix or a full VM.
  - Done when:
    - All three previous tasks pass their verification steps.
    - `cargo clippy -p cli` is clean on the changed files.
    - `cargo fmt --check` passes.
    - Context docs (`context/context-map.md`, `context/overview.md`) are synced if the architecture or key behaviours changed.
    - The plan is marked complete.
  - Verification notes (commands or checks):
    - `cargo test -p cli`
    - `cargo fmt --check --all`
    - `cargo clippy -p cli 2>&1 | grep -E "error|warning" | grep -v "generated" | wc -l` → 0
    - Manual smoke test as described in T03.

## Open questions

None — all three issues are well-understood and the tasks are scoped to specific code changes.
