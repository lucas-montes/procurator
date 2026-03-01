# CLI — User Interface

## What

The primary user-facing command-line tool (`pcr`). Provides four top-level commands:

| Command | Purpose |
|---------|---------|
| `init` | Set up a workspace from a `flake.nix` |
| `stack` | Manage local dev stack (up/down/stop/start/restart) |
| `repo` | Clone, push, pull repositories |
| `inspect` | TUI-based cluster inspection (planned, via ratatui) |

Also ships two test binaries (`pcr-test`, `pcr-worker-test`) for manually exercising the Master and Worker RPC interfaces.

## Why

Developers need a single entry point to interact with procurator. The CLI is intentionally minimal and declarative — it never allows commands that would introduce drift from the Nix-defined project spec. All configuration comes from flakes, not CLI flags.

The test binaries exist separately because RPC testing requires different workflows than normal usage. See `TESTING.md` for details.

## Status

Scaffolded — `init` has an implementation, other commands are stubs. Test binaries are functional.
