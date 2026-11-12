# CLI — User Interface

## What

The primary user-facing command-line tool (`pcr`). Provides four top-level commands:

| Command | Purpose |
|---------|---------|
| `init` | Set up a workspace from a `flake.nix` (scans the repo via `autonix` and writes config + `flake.nix`) |
| `stack` | Run the local dev stack in the foreground (`start` only — no `up`/`down`/`stop`/`restart`) |
| `vcs` | Git operations: `vcs repo {clone,pull,push,branch}` (single repo), `vcs project {clone,pull,push,branch,list}` (multi-repo with submodules), `vcs agent {prepare,list}` (agent workspace prep: clone + branch) |
| `agents` | Workspace lifecycle placeholders (`up`/`down`/`stop`/`start`/`restart`) — stubs only, see Status |

Also ships the `pcr-worker-test` binary for manually exercising worker RPC calls.

## Why

Developers need a single entry point to interact with procurator. The CLI is intentionally minimal and declarative — it never allows commands that would introduce drift from the Nix-defined project spec. All configuration comes from flakes, not CLI flags.

The test binary exists separately because RPC testing requires a different workflow than normal usage. See [`../docs/testing.md`](../docs/testing.md) for details.

## `pcr-worker-test`

Test-only binary, intentionally separated from `pcr`.

Supported verbs:

- `read`
- `list`
- `create`
- `delete`

## Status

Further along than "scaffolded": `init`, `stack start` (full service graph, supervisor, log writer, file watcher), and all of `vcs` (`repo`, `project`, `agent`) are implemented, not stubs. Test binaries are functional.

The only real stub is the top-level **`agents`** command (`pcr agents up/down/stop/start/restart`) — every arm just prints a placeholder message. Note this is a different, unrelated surface from `vcs agent` (which actually prepares an agent workspace by cloning a repo and creating a branch); the similar naming is easy to confuse and worth resolving (rename or remove one of them) independently of this audit.

`stack` itself only implements `start`; `up`/`down`/`stop`/`restart` don't exist on it.

## Stack

To test it manually

```bash
cargo run -p cli --bin pcr -- vcs repo push --path .
```
