# Plan: Per-service healthcheck

Plan name: `healthcheck`
Path: `context/plans/healthcheck.md`

## Change Summary

Add a Docker-Compose-inspired healthcheck system to the stack configuration. Each
service can declare an optional `healthcheck` block defining a command to probe
readiness/liveness. During startup, a dependent service blocks until its
dependencies pass their healthchecks. After startup, healthchecks run
periodically and log state transitions (healthy/unhealthy) — no auto-restart.

This addresses the cascading failure observed when one service dies (e.g. `nc`
exits after a connection reset from a restarted client) and its dependents
silently break.

## Success Criteria

1. A service with `healthcheck` configured runs its test command every `interval`.
2. A service's startup blocks until all its dependencies have passed their
   healthchecks (up to `retries` attempts with `timeout` per attempt).
3. Both check types work:
   - `test = ["CMD", "prog", "arg"]` — direct exec
   - `test = "shell command"` — via `sh -c`
4. Health state transitions are logged: `[name] healthcheck: passed` /
   `[name] healthcheck: failed (retry N/M)`.
5. Services without a `healthcheck` block are not affected (existing behavior).
6. `cargo build -p cli` passes, `cargo test -p cli` passes.

## Constraints and Non-goals

- **In scope:** Config schema (`config.rs`), healthcheck runner (`service.rs` or new
  `health.rs`), Supervisor integration (blocking startup + periodic checks).
- **Out of scope:** Auto-restart on healthcheck failure (log-only).
- **Out of scope:** Healthcheck status exposed via HTTP API or CLI commands.
- **Out of scope:** Readiness gates for the watcher/hot-reload path (new services from
  config changes also block on deps).
- **No new crate dependencies** — uses existing `tokio::process::Command`.

## Tasks

- [x] T01: Add `HealthCheckConfig` to the service schema (status:done)

  **Task ID:** T01
  **Goal:** Add an optional `healthcheck` field to `Service` in `config.rs`
    with the Docker-Compose-inspired shape.

  **Boundaries (in/out of scope):**
  - In:
    - Define in `config.rs`:
      ```rust
      #[derive(Debug, Clone, Serialize, Deserialize)]
      pub struct HealthCheckConfig {
          test: serde_json::Value,   // string or array of strings
          #[serde(default = "default_healthcheck_interval")]
          interval_secs: u64,
          #[serde(default = "default_healthcheck_timeout")]
          timeout_secs: u64,
          #[serde(default = "default_healthcheck_retries")]
          retries: u32,
      }

      const fn default_healthcheck_interval() -> u64 { 30 }
      const fn default_healthcheck_timeout() -> u64 { 10 }
      const fn default_healthcheck_retries() -> u32 { 3 }
      ```
    - Add `healthcheck: Option<HealthCheckConfig>` field to `Service`.
    - Add getter `Service::healthcheck() -> Option<&HealthCheckConfig>`.
    - Update `ServiceGraph::validate()` to catch invalid test formats (non-string
      non-array, empty array) as new `ParserError` variant(s).
    - Implement serde defaults for the numeric fields.
  - Out: No runtime healthcheck execution (T02). No Supervisor integration (T03).

  **Done when:** `HealthCheckConfig` defined, parseable from mock config JSON,
    invalid configs produce parser errors. `cargo build -p cli` passes.

  **Verification:**
  ```bash
  cargo build -p cli 2>&1 | grep "^error"
  ```

  **Status:** done
  **Completed:** 2026-06-18
  **Files changed:** `cli/pcr/stack/config.rs`
  **Evidence:** Build clean, 8/8 tests passed, clippy clean on changed file, fmt clean

- [x] T02: Implement healthcheck runner (status:done)

  **Task ID:** T02
  **Goal:** Create a `HealthCheckRunner` (in a new `health.rs` or in `service.rs`)
    that runs a single healthcheck command respecting `timeout`, handles both
    string and array test formats, and returns a pass/fail result.

  **Boundaries (in/out of scope):**
  - In:
    - New function or struct that:
      - Takes `&HealthCheckConfig` and `&Path` (working dir)
      - Spawns `tokio::process::Command` for the test
      - Applies `timeout` via `tokio::time::timeout`
      - Returns `Ok(())` on zero exit code, `Err` otherwise
    - Handle `test` string → `sh -c "$test"`
    - Handle `test` array → direct exec of first element with rest as args
    - Handle CMD-SHELL prefix (if array starts with `"CMD-SHELL"`, treat rest as
      string test). This matches Docker Compose's `CMD-SHELL` mode.
    - Timeout kills the child process if it exceeds `timeout_secs`
    - Integrate with `parse_cmd` or use a similar pattern
  - Out: No Supervisor integration (T03). No blocking startup behavior (T03).

  **Done when:** Runner compiles, unit-testable, handles both test formats,
    respects timeout, returns pass/fail.

  **Verification:**
  ```bash
  cargo build -p cli 2>&1 | grep "^error"
  ```

  **Status:** done
  **Completed:** 2026-06-18
  **Files changed:** `cli/pcr/stack/health.rs` (new), `cli/pcr/stack/mod.rs`
  **Evidence:** Build clean, 16/16 health tests + 8 config tests passed, clippy clean on changed files, fmt clean

- [x] T03: Integrate healthchecks into Supervisor (status:done)

  **Task ID:** T03
  **Goal:** The `Supervisor` blocks service startup until dependencies pass
    their healthchecks, and runs periodic healthchecks on all configured
    services during the event loop.

  **Boundaries (in/out of scope):**
  - In:
    - Extract a helper that polls a dependency's healthcheck: waits up to
      `retries * (interval + timeout)` for it to pass. Polls every `interval`
      seconds, kills each attempt after `timeout`.
    - In `start_all()` / startup path: before starting each service, wait for
      all its dependencies to pass healthcheck (if they have one configured).
      If a dependency has no healthcheck, skip.
    - After all services start, spawn a background healthcheck loop per service:
      every `interval` seconds, run the healthcheck. On state change
      (healthy→unhealthy or unhealthy→healthy), log it.
    - Log format: `[server] healthcheck: passed` /
      `[server] healthcheck: failed (retry 2/3)`
  - Out: No auto-restart on failure. No watcher interaction changes (the
    existing config-changed / source-changed paths remain unchanged).

  **Done when:** A service with `dependsOn = ["server"]` waits for server's
    healthcheck to pass before starting. Periodic healthcheck logs appear
    for configured services. `cargo build -p cli` passes.

  **Verification:**
  ```bash
  cargo build -p cli 2>&1 | grep "^error"
  ```

  **Status:** done
  **Completed:** 2026-06-18
  **Files changed:** `cli/pcr/stack/config.rs`, `cli/pcr/stack/service.rs`, `mock/flake.nix`
  **Evidence:** Build clean, 24/24 tests passed, clippy clean on changed files, fmt clean. Manual smoke test confirmed blocking + healthcheck logging.

- [x] T04: Mock config update and unit tests (status:done)
  - **Completed:** 2026-06-18
  - **Files changed:** `cli/pcr/stack/config.rs`, `cli/pcr/stack/health.rs`, `mock/flake.nix`
  - **Evidence:** 33/33 tests passed (was 24 before T04). 5 validation tests in config.rs, 4 new tests in health.rs. `cargo build -p cli` clean. `mock/flake.nix` has `CMD-SHELL` healthcheck for `server`.

  **Task ID:** T04
  **Goal:** Update the mock `flake.nix` to add a healthcheck for the `server`
    service, and add unit tests for parsing, validation, and the runner.

  **Boundaries (in/out of scope):**
  - In:
    - Add to `mock/flake.nix`:
      ```nix
      server = {
        cmd = ["nix" "run" "nixpkgs#python3" "--" "."];
        src = "./services/server";
        ports = [8080];
        dependsOn = ["migrate"];
        healthcheck = {
          test = ["CMD-SHELL" "ss -tln | grep -q 8080 || exit 1"];
          interval_secs = 10;
          timeout_secs = 5;
          retries = 2;
        };
      };
      ```
    - Unit tests for:
      - Parsing valid healthcheck JSON
      - Rejecting invalid test formats
      - Runner success (zero exit)
      - Runner failure (non-zero exit)
      - Runner timeout
  - Out: No integration tests (requires running mock stack).

  **Done when:** `cargo test -p cli` passes with additional healthcheck tests.
    `cargo build -p cli` passes.

  **Verification:**
  ```bash
  cargo test -p cli 2>&1 | grep -E "test result|FAILED"
  ```

- [x] T05: Validation and context sync (status:done)

  **Task ID:** T05
  **Goal:** Full build, all tests pass, fmt check, manual smoke test, context sync.

  **Boundaries (in/out of scope):**
  - In: Build, unit tests, fmt, smoke test with mock stack, context sync.
  - Out: No new feature work.

  **Done when:**
  - `cargo build -p cli` — zero errors
  - `cargo test -p cli` — all tests pass
  - `cargo fmt --all --check` — clean
  - Smoke test: start mock stack, confirm client blocks until server passes
    healthcheck, periodic healthcheck logs appear
  - Working tree clean

  **Verification:**
  ```bash
  cargo build -p cli
  cargo test -p cli
  cargo fmt --all --check
  ```

  **Status:** done
  **Completed:** 2026-06-18
  **Files changed:** (none — validation only)
  **Evidence:** See Validation Report below.

## Validation Report

### Commands run
| Command | Exit code | Result |
|---|---|---|
| `cargo test -p cli` | 0 | 33 passed, 0 failed |
| `cargo fmt --all --check` | 0 | Clean (no output) |
| `cargo build -p cli` | 0 | Clean (3 pre-existing warnings, none in healthcheck code) |

### Manual smoke test
Command: `cargo run -p cli --bin pcr -- stack --path ./mock start` (run for ~25s, killed by timeout)

Key observations from output:
1. **Blocking startup**: `waiting for dependency healthcheck service=client dependency=server` — client waits until server passes healthcheck
2. **Healthcheck retries**: `healthcheck: failed name=server attempt=1 max_retries=2` — first attempt failed because server hadn't started yet
3. **Eventually passes**: `healthcheck: passed name=server` — after ~10s when server is ready
4. **Dependent starts**: `[client] started (pid 331056)` — client starts immediately after dep passes
5. **Clean exit**: Process exited cleanly on timeout (SIGTERM)

### Success-criteria verification
- [x] Service with healthcheck runs test every `interval` → confirmed via periodic logs
- [x] Startup blocks until dependencies pass healthcheck → confirmed via smoke test
- [x] `CMD-SHELL` test format works → `ss -tln | grep -q 8080` via mock/flake.nix
- [x] State transitions logged → `healthcheck: passed` / `healthcheck: failed` visible
- [x] Services without healthcheck unaffected → `migrate` and `worker` start normally
- [x] `cargo build -p cli` passes, `cargo test -p cli` passes → confirmed

### Post-completion fixes (applied 2026-06-18)

The following bugs were found and fixed after T05 validation:

1. **Ctrl-C not handled during startup** — `start_services_with_healthchecks()` had no Ctrl-C handling because it ran before the event loop's `select!`. Fixed by wrapping the startup in a `select!` with `ctrl_c()`.

2. **False-positive healthcheck** — When a stale process (from a previous run) holds the service port, the `ss -tln | grep -q 8080` healthcheck passes even though the actual service process is dead. This allowed `client` to start after `server` crashed with "Address already in use". Fixed by adding `RunningManifest::is_alive()` and verifying the dependency process is still running after the healthcheck passes.

3. **Healthcheck loops outlive their service** — Background `spawn_healthcheck_loop` tasks ran forever even after the service exited, continuing to probe ports that might be held by other processes. Fixed by passing a `tokio::sync::Notify` stop signal to each loop; the signal fires when the service exits or is removed.

### Residual risks
- `ServiceManifest::start_all()` (line 250 in `service.rs`) is now dead code — replaced by `start_services_with_healthchecks()`. Should be removed in a future cleanup.
- The file watcher re-classifies paths on every log file write (visible in smoke test output: `classifying changed paths` flooding). The `logs/` directory inside the watch root triggers unnecessary watcher events. Not yet planned.
- No integration tests — manual smoke test only. The runner relies on `ss` being present in the container/host for `CMD-SHELL` healthchecks.
