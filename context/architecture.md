# Repohub Architecture

High-level architecture of the Repohub service (`repohub/`), focused on the DORA metrics subsystem.

## DORA Metrics Module

Defined in `repohub/src/application/dora/mod.rs`. Provides HTTP endpoints for weekly DORA metric snapshots and a periodic background refresh task.

```
┌───────────────────────────────────────────────────────┐
│                     main.rs                           │
│                                                       │
│  DoraAppState { db } ──────────────────────────┐      │
│       │                                          │      │
│       ▼                                          ▼      │
│  DORA routes (metrics_handler)     tokio::spawn background │
│  GET /{user}/{proj}/{repo}/        task ───────────────┐ │
│      dora/metrics?week=              │                   │ │
│       │                              │                   │ │
│       ▼                              ▼                   │ │
│  DB query                      RefreshOrchestrator      │ │
│  (list_weekly_metric_          ├── Box<dyn ForgeSignalPort│ │
│   snapshots_by_repository)     └── Database              │ │
└───────────────────────────────────────────────────────┘
```

## State Wiring

Two independent state ownership boundaries:

| Component | Owns | Created in |
|-----------|------|------------|
| `DoraAppState` | `Database` (clone) | `main.rs` — passed to `dora_routes().with_state()` |
| Background task | `RefreshOrchestrator` + its own `Database` clone | `main.rs` — `tokio::spawn` captures owned values |

`DoraAppState` is a lightweight `Clone` handle containing only the `Database` pool. The orchestrator is not shared — it lives exclusively inside the spawned task.

## Route Wiring

Routes are built in `main.rs` by merging three independent router trees:

```
let app = github_app.merge(gerrit_app).merge(dora_routes);
```

`dora_routes()` returns `Router<DoraAppState>` mounted at:

- `GET /{username}/{project}/{repo}/dora` — renders HTML dashboard with week picker, grouped metric tables, and Chart.js trend charts (T09)
- `GET /{username}/{project}/{repo}/dora/metrics?week=<ISO date>` — returns JSON array of `WeeklyMetricSnapshotRow`

No auth on DORA endpoints (v1). Error responses: `404 Not Found` for unknown user/project/repo, `500 Internal Server Error` for DB failures.

## Background Task Lifecycle

1. **Startup guard**: If `dora_github_owner` or `dora_github_repo` is empty, the background task is not spawned.
2. **Credential guard**: If `github_app_id == 0`, the spawned task logs a warning and returns immediately (task exits, process continues).
3. **Repository resolution**: `resolve_dora_target()` scans all users/projects/repos to find a `ForgeRepositoryTarget` by name. If not found, logs a warning and the task exits.
4. **First tick**: `interval.tick().await` fires immediately on startup for fast initial data fetch.
5. **Loop**: On each tick, calls `orchestrator.trigger_refresh(target, patterns, "v1")`. Success or failure is logged via `tracing::info!`/`tracing::error!`. The loop continues on failure (retries next interval).
6. **No concurrency guard**: The task is the sole caller of `trigger_refresh`; no mutex/lock on the orchestrator.

## Background Task Design Decisions (v1)

- **No dedicated refresh trigger endpoint** — refresh is triggered only by the periodic schedule. An explicit API trigger is deferred to a future version.
- **Periodic schedule only** — configurable via `dora_interval_seconds` (default 3600s). No webhook or event-driven trigger.
- **Error logging via tracing** — all refresh failures are logged at `error` level. No alerting, no circuit breaker, no exponential backoff (beyond the fixed-interval retry).

## Config Surface

Defined in `repohub/src/config.rs` (`Config` struct):

| Field | Type | Default | Purpose |
|---|---|---|---|
| `dora_github_owner` | `String` | `""` | GitHub owner/org for the target repository |
| `dora_github_repo` | `String` | `""` | GitHub repository name |
| `dora_interval_seconds` | `u64` | `3600` | Seconds between background refresh ticks |
| `dora_incident_label_patterns` | `Vec<String>` | `[".*incident.*"]` | Regex patterns for incident issue label detection |

## Pre-existing Compile Error Resolution (T08)

T08 fixed pre-existing compile errors that blocked `cargo build -p repohub`:

| File | Error | Fix |
|---|---|---|
| `adapters/github/auth.rs` | Duplicate `GithubAuthError` definition, missing `jsonwebtoken::Error`, missing `JwtClaims` | Consolidated error enum, added missing imports |
| `application/github/service.rs` | Dead code referencing removed `GithubApiClient`/`GithubApiError` | Deleted file |
| `domain/signals/pull_request.rs` | Test typing mismatch | Fixed test type annotation |

These fixes resolved all compilation blockers; `cargo build -p repohub` now succeeds.

## Trait Bounds

`ForgeSignalPort: Send + Sync` is required so `RefreshOrchestrator` (holding `Box<dyn ForgeSignalPort>`) can be moved across `tokio::spawn` boundaries.

## Key Files

| Path | Role |
|---|---|
| `repohub/src/application/dora/mod.rs` | DORA HTTP handler + router |
| `repohub/src/application/ports.rs` | `ForgeSignalPort` trait + `Send+Sync` bound |
| `repohub/src/application/mod.rs` | Module declarations |
| `repohub/src/lib.rs` | Crate root — re-exports `DoraAppState`, `dora_routes` |
| `repohub/src/main.rs` | State wiring, background task spawn, route merging |
| `repohub/src/config.rs` | `Config` struct with DORA fields |
| `repohub/src/services/refresh_orchestrator.rs` | `RefreshOrchestrator` pipeline |

See also: [repohub/dora-api.md](repohub/dora-api.md), [repohub/refresh-orchestrator.md](repohub/refresh-orchestrator.md), [repohub/forge-ports.md](repohub/forge-ports.md)
