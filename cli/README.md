# CLI — User Interface

## What

The primary user-facing command-line tool (`pcr`). Provides four top-level commands:

| Command | Purpose |
|---------|---------|
| `init` | Set up a workspace from a `flake.nix` |
| `stack` | Manage local dev stack (up/down/stop/start/restart) |
| `repo` | Clone, push, pull repositories |
| `inspect` | TUI-based cluster inspection (planned, via ratatui) |

Also ships the `pcr-test` binary for manually exercising worker RPC calls.

## Why

Developers need a single entry point to interact with procurator. The CLI is intentionally minimal and declarative — it never allows commands that would introduce drift from the Nix-defined project spec. All configuration comes from flakes, not CLI flags.

The test binary exists separately because RPC testing requires a different workflow than normal usage. See [`../docs/testing.md`](../docs/testing.md) for details.

## Status

Scaffolded — `init` has an implementation, other commands are stubs. Test binaries are functional.
