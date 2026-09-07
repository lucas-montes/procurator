# Healthcheck System

Healthcheck configuration and execution for stack services, inspired by Docker
Compose. Defined in `cli/pcr/stack/health.rs` and `cli/pcr/stack/config.rs`.

## Config Schema

Services may declare an optional `healthcheck` block:

```nix
server = {
  cmd = ["nix" "run" "nixpkgs#python3" "--" "."];
  healthcheck = {
    test = ["CMD-SHELL" "ss -tln | grep -q 8080 || exit 1"];
    interval_secs = 10;   # default 30
    timeout_secs = 5;     # default 10
    retries = 2;           # default 3
  };
};
```

### Test formats

| Nix value | Behaviour |
|---|---|
| `"shell command"` | `sh -c "shell command"` |
| `["CMD", "prog", "arg"]` | Direct exec of `prog` with `arg` (CMD prefix stripped) |
| `["CMD-SHELL", "cmd"]` | `sh -c "cmd"` (CMD-SHELL prefix stripped) |
| `["prog", "arg"]` | Direct exec (no prefix) |

## Runner API

```rust
pub async fn run_healthcheck(
    config: &HealthCheckConfig,
    working_dir: &Path,
) -> Result<(), HealthCheckError>
```

`HealthCheckError` variants:
- `Spawn(String)` — command could not be started
- `Timeout` — command exceeded `timeout_secs`
- `Failed(i32)` — command exited non-zero
- `ParseCmd` — invalid test format

## Current Status (T05 — Complete, with post-completion fixes)

- Schema defined and validated (T01)
- Runner implemented and unit-tested (T02)
- Supervisor integration (T03):
  - `wait_for_dependency_health()` — polls a dependency's healthcheck up to `retries` times during startup, blocking the dependent service until it passes
  - `start_services_with_healthchecks()` — iterates services in dependency order; before each service starts, waits for all dependencies with healthchecks to pass
  - `spawn_healthcheck_loop()` — per-service background task that runs healthchecks every `interval_secs` and logs state transitions via the `LogLine` channel
  - `spawn_periodic_healthchecks()` — called after all services start, spawns loops for every service with a healthcheck
- Mock config and unit tests added (T04):
  - `mock/flake.nix` has `CMD-SHELL` healthcheck for `server`
  - 5 validation tests in `config.rs`, 4 new runner tests in `health.rs` (33 total)
- Validation completed (T05), smoke test confirmed blocking startup and periodic checks
- Post-completion fixes (2026-06-18):
  - **Ctrl-C during startup**: `start_services_with_healthchecks()` now wrapped in `select!` with `ctrl_c()` so the user can cancel during blocking healthcheck waits
  - **False-positive guard**: After a dependency's healthcheck passes, `RunningManifest::is_alive()` verifies the dependency process is still running before allowing the dependent to start. Catches stale-port scenarios where a dead process holds the port.
  - **Cancellable healthcheck loops**: Each `spawn_healthcheck_loop()` receives a `tokio::sync::Notify` stop signal. When a service exits or is removed, its healthcheck loop is cancelled, preventing stale checks against orphaned ports.

See also: [lifecycle.md](lifecycle.md), [context-map.md](../context-map.md)
