# `pcr stack` — Architecture Evolution

## Current Architecture (as of June 2026)

Foreground-only supervisor (`start` only, Ctrl-C to stop). No state file, no cross-terminal
`stop` command. This follows **Approach E (Foreman-style)** from the original analysis.

## Key differences from the original architecture

| Aspect | Original (v1) | Current (v2) |
|--------|---------------|---------------|
| Commands | `Start`, `Stop` | `Start` only |
| Stop mechanism | Cross-terminal via state file | Ctrl-C only |
| State persistence | `.pcr-stack/state.json` with `fs2` locking | None |
| Architecture pattern | Ports-and-adapters (hexagonal) | Simplified manifest-based |
| Error handling | `Result<T, String>` | Typed error enums per module |
| Service lifecycle | `RunningService` struct | Type-state `Service<Parsed>` / `Service<Running>` |
| Config parsing | Three separate `nix eval` calls | Single `nix eval .#stack` call |
| Configuration types | Spread across `parser.rs` | Consolidated in `config.rs` |
| Supervision | `ProcessSupervisor` in `process.rs` + traits in `supervisor.rs` | `Supervisor` event loop in `service.rs` |

## Current module architecture

```
CLI (cli.rs)  ──spawns──►  Supervisor (service.rs)
   │                            │
   │                       tokio::select! {
   │                         ctrl_c()      → graceful shutdown
   │                         watcher_rx    → diff & restart
   │                       }
   │                            │
   └──spawns──►  Watcher (watch.rs)  ──sends manifest──►
                Log Writer (logging.rs)
```

## Why state file was removed

1. **Simplicity**: Ctrl-C is the natural stop mechanism for a foreground process. A state
   file adds file I/O, locking, staleness detection, and crash recovery complexity.
2. **Scope**: Cross-terminal `stop` was never used in practice during mock testing.
3. **Foreman precedent**: `foreman start` has been the reference for dev process management
   for over a decade — no separate stop needed.
4. **Watch mode fills the gap**: Hot-reload covers the primary use case for restarting
   services without restarting the stack.

## Type-state pattern

Services use a type-state pattern to prevent invalid operations at compile time:

```rust
Service<Parsed>  // Config resolved, ready to start
    │
    │ .start(logs_tx)
    ▼
Service<Running> // Child process running, can be killed
    │
    │ .kill()
    ▼
    (dropped)
```

- `Service<Parsed>` has no `kill()` method — you can't kill something that hasn't started.
- `Service<Running>` has no `start()` method — you can't start something already running.

## ServiceManifest as currency type

`ServiceManifest` bridges the config layer and the runtime layer:

```
config::ServiceGraph  ──►  ServiceManifest  ──►  RunningManifest
  (config.rs)          from_graph()    start_all()    (running children)
                              │
                              │ .diff(other_manifest)
                              ▼
                         HashMap<String, ServiceChange>
```

The `diff()` method compares two manifests by `cmd` field and returns Added / Removed /
Changed / Unchanged. The `Supervisor` uses this to apply hot-reload changes.

## Signal handling

- `tokio::signal::ctrl_c()` in the `Supervisor::run()` event loop.
- On signal: calls `RunningManifest::kill_all()` in reverse dependency order.
- SIGTERM sent via `tokio::process::Child::kill()` (which sends SIGKILL on Unix).
- No graceful escalation (SIGTERM → wait → SIGKILL) — the original escalation was
  removed when the state file was dropped. Can be re-added if needed.

## oneShot services

- Run synchronously during `ServiceManifest::start_all()`.
- If a oneShot service fails (non-zero exit), the error is logged but the stack continues.
- oneShot services are **not** re-executed during watch-mode hot-reload.

## File watcher

- The `Watcher` watches the repo root recursively.
- On any file change: re-parses `nix eval .#stack` and sends a fresh `ServiceManifest`.
- 200ms debounce to coalesce editor save flurries.

## Validation

| Check | Location | Description |
|-------|----------|-------------|
| Cycle detection | `config.rs` | DFS-based cycle detection in `has_cycle()` |
| Port validation | `config.rs` | Rejects port 0, rejects duplicate ports |
| Dependency validation | `config.rs` | Rejects dependsOn references to unknown services |
| Cmd validation | `config.rs` | Non-oneShot services must have a non-null `cmd` |
| Cmd parsing | `service.rs` | Rejects non-string non-array `cmd` values |

## Historical decisions

| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-06-01 | Approach A (state file) adopted | Cross-terminal `stop`, ports/adapters for future daemon |
| 2026-06-11 | Switched to Approach E (Foreman-style) | Simplified after watch mode made hot-reload primary use case |

See also: [module-layout.md](module-layout.md) for the current module structure,
[stack-nix-schema.md](../specs/stack-nix-schema.md) for the Nix schema.
