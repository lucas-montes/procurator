# CI Service — Flake Checks

## What

Runs every check a repository's flake defines whenever someone pushes,
reusing the binary cache so already-built checks cost nothing. Captures each
check's log for users and sends events (failure, success, recovery) to
webhooks and the deploy target.

The target design is in [`context/specs/ci-service.md`](../context/specs/ci-service.md).

## Layout

Hexagonal, like `cache`:

- `src/domain/`: pure rules. `run.rs` covers run identity (`RepoName`,
  `CommitHash`, `GitRef`, `Pusher`), statuses, and the allowed lifecycle
  transitions.
- `src/application/`: `Service` with every use case, plus `ports.rs`
  (`RunStore`, `Clock`).
- `src/adapters/`: `sqlite.rs` and `memory.rs` (run stores, both held to
  `store_contract.rs`), `system.rs` (clock), `http/` (axum router).
- `src/main.rs`: composition root for a standalone deployment. In the
  monolith, other crates hold `ci_service::application::Service` and call it
  directly.

## HTTP API

| Method | Path | |
|---|---|---|
| POST | `/api/runs` | `{"repo":"owner/name","commit":"<sha>","ref":"refs/heads/main","pusher":"git"}` → 202 `{"id":1,"status":"queued"}` |
| GET | `/api/runs/{id}` | one run |
| GET | `/api/repos/{owner}/{name}/runs?before=<id>&limit=<n>` | newest first, default 20, max 100 |
| GET | `/health` | `OK` |

The post-receive hook lives in `nix/modules/repohub/post-receive`.

## Run locally

```bash
cargo run -p ci_service -- --config ./ci_service/example.toml
```

## Status

Build step 1 of the spec is done: domain types, ports, in-memory and `SQLite`
run stores, atomic claim (`UPDATE … RETURNING`), recovery of runs left
`running` at startup, and the HTTP API.

**Runs are queued but not executed yet.** The worker that evaluates and builds
checks is step 2. Until then a queued run stays `queued`.
