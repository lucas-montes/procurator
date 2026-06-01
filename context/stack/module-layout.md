# Stack CLI Module Layout

Internal module structure of `cli/pcr/stack/` after the lifecycle improvement
(start/stop + signal handling + state file).

## Module Map

```
cli/pcr/stack/
├── mod.rs         # Module declarations, pub use StackArgs
├── cli.rs         # StackArgs (clap Args), StackCommands enum, dispatch
├── parser.rs      # Service, ServiceGraph, parsing, topological sort
├── supervisor.rs  # Port trait interfaces + data types + FileStackState adapter
└── process.rs     # ProcessSupervisor adapter (process lifecycle, signal handling)
```

## File Responsibilities

| File | Responsibility |
|---|---|
| `mod.rs` | Declares submodules (`cli`, `parser`, `supervisor`). Re-exports `StackArgs` as the public API. |
| `cli.rs` | Defines `StackCommands` enum (`Start`, `Stop`), `StackArgs` with `--path` flag, `execute()` dispatch. |
| `parser.rs` | Defines `Service`, `ServiceGraph`, `ServiceGraph::validate()`, `has_cycle()`, `parse_flake_services()`, `topo_sort()`, and the run logic. |
| `supervisor.rs` | Defines port trait interfaces (`StackState`, `ServiceSupervisor`), data types (`RunningStack`, `RunningService`, `ServiceStatus`), and the `FileStackState` adapter (file-based `StackState` implementation with `fs2` advisory locking). |
| `process.rs` | Defines `ProcessSupervisor` — the `ServiceSupervisor` adapter that spawns child processes, streams logs, handles SIGINT/SIGTERM with graceful escalation (SIGTERM → 5s → SIGKILL), and manages oneShot services. Uses `tokio::signal::unix` for signal handling. |

## Key Type Locations

- `Service` — struct in `parser.rs` with serde deserialization for `stack.services` JSON
- `ServiceGraph` — struct in `parser.rs` with `services: HashMap<String, Service>` and `order: Vec<String>`
- `StackCommands` — enum in `cli.rs` (`Start`, `Stop` variants)
- `StackState` — trait in `supervisor.rs` (port for persisting/loading running stack state)
- `ServiceSupervisor` — trait in `supervisor.rs` (port for process lifecycle)
- `RunningStack` — struct in `supervisor.rs` (serialisable snapshot of running services)
- `ServiceStatus` — enum in `supervisor.rs` (`Running`, `Stopped`, `Failed`)
- `FileStackState` — struct in `supervisor.rs` implementing `StackState` via `.pcr-stack/state.json` with `fs2` locking
- `ProcessSupervisor` — struct in `process.rs` implementing `ServiceSupervisor` (spawn, signal handling, graceful shutdown)

## Architecture

Ports-and-adapters (hexagonal) pattern:

```
CLI layer (cli.rs)         — dispatches Start/Stop
      │
Port layer (supervisor.rs) — StackState + ServiceSupervisor traits
      │
Adapter layer              — FileStackState + ProcessSupervisor (current)
                              └─ Future: SocketStackState + DaemonSupervisor
```

See also: [stack-nix-schema.md](../specs/stack-nix-schema.md) for the Nix schema,
[stop-start-architecture.md](stop-start-architecture.md) for the architecture decision.
