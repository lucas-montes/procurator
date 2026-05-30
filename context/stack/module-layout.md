# Stack CLI Module Layout

Internal module structure of `cli/pcr/stack/` after the T01-T02 refactoring.

## Module Map

```
cli/pcr/stack/
├── mod.rs         # Module declarations, Service/ServiceGraph types, validation
├── cli.rs         # StackArgs (clap Args), StackCommands dispatch
├── commands.rs    # StackCommands (clap Subcommand enum)
└── parser.rs      # Parsing, topological sort, service execution
```

## File Responsibilities

| File | Responsibility |
|---|---|
| `mod.rs` | Declares submodules (`commands`, `parser`, `cli`). Defines `Service`, `ServiceGraph`, `ServiceGraph::validate()`, and the `has_cycle()` helper. Re-exports `StackArgs` as the public API. |
| `cli.rs` | Defines `StackArgs` with `--path` flag and dispatches to `StackCommands` via `execute()`. Calls `parser::parse_and_run()` for the `Up` command. |
| `commands.rs` | Defines the `StackCommands` enum (`Up`, `Down`, `Stop`, `Start`, `Restart`) with clap derive. No behavior. |
| `parser.rs` | Contains `parse_and_run`, `parse_flake_services`, `topo_sort`, `run_stack`, `run_stack_async`. |

## Key Type Locations

- `Service` — struct in `mod.rs` with serde deserialization for `stack.services` JSON
- `ServiceGraph` — struct in `mod.rs` with `services: HashMap<String, Service>` and `order: Vec<String>`
- `StackCommands` — enum in `commands.rs` imported by `cli.rs` via `super::commands::StackCommands`
- Parser functions — all in `parser.rs`, imported by `cli.rs` via `super::parser::parse_and_run`

See also: [stack-nix-schema.md](../specs/stack-nix-schema.md) for the Nix schema and runtime behavior.
