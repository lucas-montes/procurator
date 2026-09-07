# Stack CLI Module Layout

Internal module structure of `cli/pcr/stack/` after the type-state refactor.

## Module Map

```
cli/pcr/stack/
├── mod.rs         # Module declarations, pub use StackArgs
├── cli.rs         # StackArgs (clap Args), execute() dispatch (Start only)
├── config.rs      # Service, ServiceGraph, parsing, validation, topo-sort
├── service.rs     # Type-state services, manifest, supervisor event loop
├── logging.rs     # LogWriter trait, TerminalWriter, FileWriter, BothWriter
└── watch.rs       # Watcher: file watcher, re-parses config, sends manifests
```

## File Responsibilities

| File | Responsibility |
|------|---------------|
| `mod.rs` | Declares submodules (`cli`, `config`, `service`, `logging`, `watch`). Re-exports `StackArgs` as the public API. |
| `cli.rs` | Defines `StackArgs` with `--path` flag, `execute()` dispatch for `Start`. Wires three tasks: log writer, watcher (optional), supervisor (foreground). |
| `config.rs` | All configuration types (`Service`, `ServiceGraph`, `LogConfig`, `WatchConfig`). Single `nix eval .#stack` call for parsing. `diff_graphs()`, `topo_sort()`, cycle detection, port/dependency validation. `ParserError` typed enum. |
| `service.rs` | Type-state `Service<Parsed>` / `Service<Running>`. `ServiceManifest` (config→runtime bridge with `diff()`). `RunningManifest` (running children). `Supervisor` event loop (`tokio::select!` on Ctrl-C + watcher channel). `Error` typed enum. |
| `logging.rs` | `LogWriter` trait with batched writer loop. `TerminalWriter` (locked stdout/stderr), `FileWriter` (rotation), `BothWriter` (composes both). `LogLine` with hashed-color `ColoredPrefix`. |
| `watch.rs` | `Watcher` — watches repo root recursively, re-parses config on file changes, sends fresh `ServiceManifest` through channel. 200ms debounce. |

## Key Type Locations

- `Service` — struct in `config.rs` with serde deserialization for `stack.services` JSON
- `ServiceGraph` — struct in `config.rs` with `services` and `order`, constructed via `from_services()`
- `WatchConfig`, `LogConfig` — structs in `config.rs` parsed from `stack.watch` / `stack.logs`
- `ParserError` — enum in `config.rs` (NixEval, JsonDecode, Io, CycleDetected, PortInvalid, etc.)
- `Service<Parsed>` / `Service<Running>` — type-state structs in `service.rs`
- `ServiceManifest` — struct in `service.rs` (currency type: config → runtime, carries `diff()`)
- `RunningManifest` — struct in `service.rs` (owns child handles, `kill_all()`, `remove()`, `insert()`)
- `Supervisor` — struct in `service.rs` (event loop with manifest diffing and restart)
- `LogWriter` — trait in `logging.rs` with `TerminalWriter`, `FileWriter`, `BothWriter` implementations

## Architecture

Foreground-only supervisor with manifest-based lifecycle:

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

- No state file, no cross-terminal `stop` (Ctrl-C only, Foreman-style).
- Configuration is parsed once at startup; watcher re-parses on file changes and sends new manifests.
- Service type-state (`Parsed` → `Running`) prevents invalid operations at compile time.

See also: [stack-nix-schema.md](../specs/stack-nix-schema.md) for the Nix schema,
[stop-start-architecture.md](stop-start-architecture.md) for the architecture evolution.
