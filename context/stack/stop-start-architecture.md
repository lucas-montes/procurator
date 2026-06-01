# `pcr stack` — Architecture Approaches for Start/Stop

## Problem

`pcr stack up` runs services in the foreground. We need `start` and `stop` to work as
separate CLI invocations (e.g. from another terminal). The question is how these
commands discover and control the running services.

## How other tools do it

### Foreman (Ruby, ~2010 — simplest reference)

- **Single command**: `foreman start` — foreground only.
- Reads `Procfile`, spawns all processes, interleaves logs to stdout.
- Signal handling via **self-pipe pattern**: traps INT/TERM/HUP, queues signals, processes
  them in the main event loop.
- On shutdown: sends SIGTERM to all children, waits N seconds (configurable via `--timeout`,
  default 5s), then SIGKILL survivors.
- **No separate `stop` command.** Stop = Ctrl-C on the running process.
- For production, `foreman export` generates systemd/upstart configs instead.
- **Key takeaway**: Foreground-only is fine for dev. No cross-terminal stop needed at this
  level — just use Ctrl-C.

### devenv (Rust, 2026 — most sophisticated)

- `devenv up` — foreground, runs all processes with TUI.
- `devenv processes` subcommand group: `list`, `status`, `logs`, `restart`, `start`, `stop`.
- **Detach/attach model**: `devenv up -d` spawns a daemon via **re-exec** (not fork, which
  is unsafe in multithreaded Rust programs).
- Communication via Unix socket (either `process-compose` or native process manager).
- The native process manager is a full supervisor: restart policies, readiness probes,
  exec/HTTP/systemd-notify health checks, socket activation, file watching, port allocation.
- **Key takeaway**: For a full-featured tool, a daemon + IPC is the right approach. But
  it's a lot of machinery for a v2 improvement.

### process-compose (Go)

- Foreground TUI or detached mode.
- Configuration is YAML; communication via Unix socket / TCP.
- Has shutdown hooks, daemon process support, dependency chains.
- Used as backend by `services-flake` and `process-compose-flake`.
- **Key takeaway**: The daemon/socket pattern is well-established in the Go/Rust ecosystem
  for process managers. But it's a separate binary — proxied by the CLI.

### Summary table

| Tool | Foreground | Separate stop | Mechanism |
|---|---|---|---|
| Foreman | ✅ | ❌ (Ctrl-C only) | In-process signal handling |
| devenv | ✅ | ✅ (processes stop) | Daemon re-exec + Unix socket |
| process-compose | ✅ | ✅ | Unix socket / TCP to daemon |

---

## Approaches for Procurator

### Approach A — State file (`.pcr-stack/state.json`)

**How it works:**
- `start` writes a state file `<repo-root>/.pcr-stack/state.json` with service configs, PIDs, status.
- `start` installs Ctrl-C/SIGTERM handler that gracefully stops children and cleans up the file.
- `stop` reads the state file, sends SIGTERM to each child PID, waits N seconds, SIGKILLs survivors, removes the file.
- State file retains service config between stop→start cycles (avoids re-evaluating Nix).

**Example state file:**
```json
{
  "version": 1,
  "services": {
    "db": { "cmd": ["postgres", "-D", "/var/lib/postgres"], "pid": 23456, "status": "running", "started_at": "..." },
    "api": { "cmd": ["cargo", "run"], "pid": 23457, "status": "running", "started_at": "..." }
  }
}
```

| Pro | Con |
|---|---|
| Explicit, inspectable state | File must be kept in sync (stale if `start` crashes) |
| Works across terminals | Race conditions on concurrent writes (need locking) |
| Enables future `ps`, per-service restart | Adds file I/O |
| Survives `start` process restart | |

### Approach B — Process group signaling

**How it works:**
- `start` creates a new process group for all child services.
- `stop` uses `kill(-pgid, SIGTERM)` to signal the entire process group.
- `start` is simply re-run (re-reads Nix config).

| Pro | Con |
|---|---|
| Zero state management | Cannot distinguish multiple stacks |
| Simple to implement | No per-service granularity |
| No stale state risk | Fragile PID matching |

### Approach C — Unix socket daemon

Like **devenv's** model: `start` launches a background supervisor daemon, communicates
via Unix socket at `.pcr-stack/control.sock`. The daemon owns all child processes.

| Pro | Con |
|---|---|
| Full process supervision | Significant complexity for v2 |
| Per-service granularity | Daemon lifecycle management |
| No stale state | Overkill for current scope |

### Approach D — PID file + signal delegation

`start` writes its own PID to `.pcr-stack/start.pid`. `stop` reads the PID and sends
SIGTERM to the `start` process, which has a handler that gracefully shuts down all
children (same code path as Ctrl-C).

| Pro | Con |
|---|---|
| Shutdown logic lives in one place | `stop` can't act if `start` not running |
| Simpler than full state file | No per-service visibility |
| Handles Ctrl-C and `stop` uniformly | |

### Approach E — Foreman-style (keep `start` only)

`start` is foreground-only. Ctrl-C to stop. No separate `stop` command.

| Pro | Con |
|---|---|
| Simplest possible codebase | Cannot stop from another terminal |
| No state management | No restart without re-evaluating Nix |

---

## Recommendation

**Start with Approach A (state file).** Here's the rationale:

1. **Foreman proves** that foreground-only is viable, but having `stop`/`start` as
   separate commands is more user-friendly (especially when running in tmux/split terminals).
2. **devenv proves** that daemon+IPC is the gold standard — but it took them years to
   build it, and even they used process-compose as a backend before writing their own.
3. **State file is the pragmatic middle ground**: it gives us cross-terminal `stop`/`start`
   without daemon lifecycle complexity. It's what many simple process managers use (PID
   files have been the Unix standard for decades).
4. The state file also acts as lightweight persistent state, avoiding redundant Nix
   evaluation on `start` after `stop`.

### Revised command names

Since the user wants `start` and `stop` (not `up`), the mapping is:
- `pcr stack start` = reads Nix config, starts all services, streams logs (foreground)  
- `pcr stack stop` = reads state file, kills all services (cross-terminal)
- ~~`up`~~ → removed (replaced by `start`)
- ~~`down`~~ → removed
- ~~`restart`~~ → removed

### Staleness mitigation

- Advisory lock on the state file itself via `fs2::FileExt::try_lock_exclusive` (no separate `.lock` file).
- `stop` checks PIDs exist before signaling (`kill -0`).
- `start` checks for stale state on startup (stack PID no longer alive) and cleans up automatically.
- Crash-safe writes: write to temp file, `sync_all`, then rename over the state file.

---

## Decision

**Approach A (state file) adopted** — implemented in `context/plans/stack-lifecycle-improvement.md` (T01–T07).

### Implementation details

The architecture uses a **ports-and-adapters (hexagonal)** pattern to allow swapping the
file-based state for a future daemon-based supervisor:

```
CLI layer (cli.rs)         — dispatches Start/Stop
      │
Port layer (supervisor.rs) — StackState + ServiceSupervisor traits
      │
Adapter layer              ─┬─ FileStackState   (state file, current)
                            └─ ProcessSupervisor (process lifecycle, current)
                              ── Future: SocketStackState + DaemonSupervisor
```

### Key modules

| File | Role |
|---|---|
| `cli/pcr/stack/supervisor.rs` | Trait interfaces (`StackState`, `ServiceSupervisor`) + data types (`RunningStack`, `RunningService`, `ServiceStatus`) + `FileStackState` adapter |
| `cli/pcr/stack/process.rs` | `ProcessSupervisor` adapter (spawn, signal handling, graceful shutdown) |
| `cli/pcr/stack/parser.rs` | `ServiceGraph`, `parse_flake_services()`, validation, topo-sort |
| `cli/pcr/stack/cli.rs` | CLI dispatch (`Start`, `Stop` commands) |

### State file locking

- The **state file itself** (`state.json`) is the lock — no separate `.lock` file.
- `fs2::FileExt::try_lock_exclusive` on the state file prevents concurrent writes.
- Lock is released when the file handle drops.
- On stale state (stack PID no longer alive), the file is cleaned up automatically.

### Signal handling

- `ProcessSupervisor::start()` installs handlers for SIGINT and SIGTERM via
  `tokio::signal::unix`.
- Graceful escalation: SIGTERM → 5s poll → SIGKILL.
- Same `kill_service_pids()` helper used for both in-process Ctrl-C and
  cross-terminal `pcr stack stop`.

### Key design decisions

| Decision | Choice |
|---|---|
| State persistence | JSON file at `.pcr-stack/state.json` |
| File locking | `fs2` advisory lock on the state file itself |
| Stale detection | `kill -0` on recorded `stack_pid` |
| Crash-safe writes | Write to temp file → `sync_all` → rename |
| Signal handling | `tokio::signal::unix` (SIGINT + SIGTERM) |
| Grace period | 5 seconds (foreman default) |
| oneShot services | Run synchronously, abort stack on failure |
| oneShot-only stack | Exit immediately (no signal-wait hang) |
