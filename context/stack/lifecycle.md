# Service Lifecycle (Supervisor)

The supervisor in `cli/pcr/stack/service.rs` manages child process lifecycle.

## Health checking

The supervisor's `select!` loop includes a 1-second tick that calls `RunningManifest::check_health()` on every iteration. This polls each child via `Child::try_wait()`:

- If `try_wait()` returns `Some(ExitStatus)` → the service has exited unexpectedly. It is removed from the running set and reported to the supervisor.
- If `try_wait()` returns `None` → the service is still alive.
- If `try_wait()` errors → the error is logged (warn level) and the service is kept for the next tick.

Detection latency is ≤1 second (the tick interval).

## Auto-restart

When a service exits unexpectedly, the supervisor looks up its config in the service manifest. If `restart == "always"` (parsed from the `stack.services.<name>.restart` flake field), the supervisor:

1. Logs a warning with the exit status
2. Waits 1 second (backoff)
3. Re-spawns the service from the manifest config
4. Logs success or failure

Services without `restart: "always"` are not restarted (including oneShot services).

## Kill paths

- **Normal shutdown** (`kill_all`): iterates services in reverse dependency order, removes from map, calls `kill()`, logs errors if kill fails (T01).
- **Single service removal** (`remove`): removes from map, calls `kill()`, logs errors (T01).
- **Supervisor death** (`SIGKILL`): on Linux, `prctl(PR_SET_PDEATHSIG, SIGTERM)` (set in `pre_exec` via T02) ensures children receive SIGTERM. On all platforms, `kill_on_drop(true)` handles normal drop / panic paths (T02).

## Key files

- `cli/pcr/stack/service.rs` — `RunningManifest::check_health()`, supervisor select loop
- `cli/pcr/stack/config.rs` — `Service::restart()`, `Service::is_one_shot()` getters

See also: [module-layout.md](module-layout.md), [overview.md](../overview.md)
