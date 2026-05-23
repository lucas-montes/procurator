# Repohub Refresh Orchestrator (T07)

Orchestrates the on-demand async refresh pipeline: fetch (via forge port) → persist normalized signals → compute per-week metrics → persist snapshots.

Defined in `repohub/src/services/refresh_orchestrator.rs`.

## Structure

```
RefreshOrchestrator
├── port: Box<dyn ForgeSignalPort>
├── db: Database
└── trigger_refresh(target, incident_label_patterns, metric_version) -> Result<RefreshResult, RefreshError>
```

The orchestrator holds a **forge-agnostic port** (`Box<dyn ForgeSignalPort>`) and a **database handle**, making it usable with any forge adapter implementation.

## Pipeline

```mermaid
flowchart LR
  A[trigger_refresh called] --> B[port.fetch_all(target)]
  B --> C{All signals empty?}
  C -->|Yes| D[Err NoData]
  C -->|No| E[persist_signals upsert]
  E --> F[detect_week_windows]
  F --> G[For each week: compute + upsert snapshot]
  G --> H[Return RefreshResult]
```

### Step-by-step

1. **Fetch** — calls `port.fetch_all(target)` to get a `NormalizedSignalBatch` with all 5 signal types (PRs, reviews, commits, deployments, issues).
2. **Validate** — if every signal list is empty, returns `RefreshError::NoData` immediately (no partial state is persisted).
3. **Persist** — upserts each non-empty signal batch into `normalized_signals` table via dedicated database methods. Idempotent via `ON CONFLICT` on `(repository_id, signal_type, source_key)`.
4. **Detect weeks** — calls `detect_week_windows` to find all distinct UTC week windows (`[Monday 00:00, next Monday 00:00)`) spanned by signal timestamps.
5. **Compute + persist** — for each week, runs `WeeklyMetricEngine::compute`, serializes the snapshot, upserts into `weekly_metric_snapshots` keyed on `(repository_id, week_start_utc, metric_version)`.
6. **Return** — `RefreshResult` with signal counts, per-week summaries, and total weeks covered.

## Key Types

| Type | Purpose |
|---|---|
| `RefreshResult` | Top-level summary: `repository_id`, `weeks_covered`, `week_summaries: Vec<RefreshWeekSummary>`, `signal_counts: RefreshSignalCounts` |
| `RefreshWeekSummary` | Per-week: `week_start_utc` (RFC3339 string), `signal_count`, `snapshot_persisted` |
| `RefreshSignalCounts` | Counts per type: `pull_requests`, `reviews`, `commits`, `deployments`, `issues` |
| `RefreshError` | `Forge(ForgeError)` — port call failed; `Database(DatabaseError)` — persistence failure; `NoData { repository_id }` — all signals empty |

## Week Detection

`detect_week_windows(batch)` finds all contiguous UTC week windows covered by signal timestamps:

- Collects every primary timestamp from all 5 signal types (PR opened, review submitted, commit committed, deployment created, issue opened).
- Finds `min` and `max` timestamps across all signals.
- Floors both to the preceding Monday 00:00:00 UTC via `floor_to_week_start`.
- Iterates from start Monday to end Monday in 7-day steps, collecting each window anchor.

`floor_to_week_start` implementation:
- Uses `weekday().num_days_from_monday()` to subtract days since Monday.
- Subracts seconds since midnight to floor to 00:00:00.
- Zeroes nanoseconds.

## Idempotency

- **No in-memory dedup guard** — calling `trigger_refresh` concurrently is not prevented (no concurrent-run guard; v1 design choice).
- **Storage-level idempotency** — all writes use `ON CONFLICT` upsert semantics:
  - `normalized_signals`: upsert on `(repository_id, signal_type, source_key)`
  - `weekly_metric_snapshots`: upsert on `(repository_id, week_start_utc, metric_version)`
- Re-triggering with identical data produces the same signal/snapshot row count (verified by test `trigger_refresh_is_idempotent`).

## Error Handling

- `ForgeError` propagates immediately from `port.fetch_all` — no partial state is committed.
- `DatabaseError` propagates from any upsert call — pipeline aborts on first DB failure.
- `NoData` error when all signal lists are empty — no writes occur. Checked before persist step.
- All stages emit structured `tracing` logs at `info`/`warn` level for observability.

## Concurrency

**No concurrency guard.** The orchestrator does not implement a mutex, lock, or queue. If `trigger_refresh` is called concurrently for the same repository, multiple pipelines may run in parallel. This is a v1 acknowledged limitation — expected usage is sequential trigger via a single API endpoint or a single periodic background task.

### Thread safety

`ForgeSignalPort: Send + Sync` is required so `RefreshOrchestrator` (which holds `Box<dyn ForgeSignalPort>`) can be used across `tokio::spawn` boundaries. The `GithubClient` adapter and all test mock ports satisfy `Send + Sync`.

## Tests

Located in `services/refresh_orchestrator.rs` `#[cfg(test)] mod tests`:

| Test | What it covers |
|---|---|
| `trigger_refresh_persists_signals_and_snapshots` | Full pipeline with all 5 signal types; verifies DB state after refresh |
| `trigger_refresh_is_idempotent` | Two sequential refreshes produce same row counts |
| `trigger_refresh_propagates_port_failure` | `ForgeError::Upstream` surfaces as `RefreshError::Forge` |
| `trigger_refresh_errors_on_empty_data` | Empty batch returns `RefreshError::NoData` |
| `week_detection_spans_multiple_windows` | Signals spanning 3 weeks produce 4 weekly snapshots |
| `floor_to_week_start_rounds_correctly` | Edge cases: Monday noon, Wednesday, Sunday 23:59, Monday midnight |

See also: [forge-ports.md](forge-ports.md), [weekly-metrics-engine.md](weekly-metrics-engine.md), [persistence.md](persistence.md), [normalized-signals.md](normalized-signals.md)
