# Repohub DORA Metrics HTTP API (T08)

DORA metric read API with periodic background refresh, defined in `repohub/src/application/dora/mod.rs`.

## State

`DoraAppState` holds a `Database` handle for querying persisted metric snapshots. The `RefreshOrchestrator` is owned by a separate background task.

## Endpoint

### GET `/{username}/{project}/{repo}/dora/metrics?week=<ISO date>`

Returns a JSON array of `WeeklyMetricSnapshotRow` objects for the given repository.

| Parameter | Location | Required | Description |
|---|---|---|---|
| `username` | path | yes | Repohub username |
| `project` | path | yes | Project name |
| `repo` | path | yes | Repository name |
| `week` | query | no | ISO week start date (e.g. `2026-05-04`); filters snapshots by week prefix match |

When `week` is absent, returns up to 52 most recent snapshots.

**Response shape:**
```json
[
  {
    "id": 1,
    "repository_id": 1,
    "week_start_utc": "2026-05-04T00:00:00+00:00",
    "metric_version": "v1",
    "metrics_json": "{...}",
    "window_days": 7,
    "computed_at": "2026-05-12T00:00:00+00:00",
    "updated_at": "2026-05-12T00:00:00+00:00"
  }
]
```

The `metrics_json` field contains the serialized `WeeklyMetricSnapshot` payload (consumer parses it client-side).

Error responses: `404 Not Found` for unknown user/project/repo, `500 Internal Server Error` for DB failures. Error details are logged via `tracing::error!`.

## Periodic Background Refresh

A `tokio::spawn`-ed task calls `RefreshOrchestrator::trigger_refresh` on a configurable interval (default: 3600 seconds).

**Startup behavior:**
- First tick fires immediately to fetch data ASAP.
- If GitHub App credentials are missing (`github_app_id == 0`), logs a warning and exits without crashing.
- If the configured repository is not found in the DB, logs a warning and exits.
- All refresh failures are logged via `tracing::error!` and the task continues to retry on the next interval.

**Configuration** (in `Config`):
- `dora_github_owner` — GitHub owner/org
- `dora_github_repo` — GitHub repository name
- `dora_interval_seconds` — interval between refreshes (default 3600)
- `dora_incident_label_patterns` — label regex patterns for incident detection (default `[".*incident.*"]`)

See also: [refresh-orchestrator.md](refresh-orchestrator.md), [forge-ports.md](forge-ports.md), [persistence.md](persistence.md)
