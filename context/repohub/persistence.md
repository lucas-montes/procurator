# Repohub Persistence Model (T05)

Canonical persistence layout for normalized ingestion signals and weekly metric snapshots.

## Tables

### `normalized_signals`
- Purpose: append/update normalized forge signals with idempotent upserts.
- Uniqueness key: `(repository_id, signal_type, source_key)`
- Important columns:
  - `repository_id`
  - `signal_type` (`pull_request|review|commit|deployment|issue`)
  - `source_key` (source-native ids/SHA)
  - `occurred_at` (event timestamp)
  - `payload_json` (serialized normalized signal payload)
  - `ingested_at`, `updated_at`
- Indexes:
  - `idx_normalized_signals_repo_time(repository_id, occurred_at)`

### `weekly_metric_snapshots`
- Purpose: store computed weekly metric bundles for single-repo reads.
- Uniqueness key: `(repository_id, week_start_utc, metric_version)`
- Important columns:
  - `repository_id`
  - `week_start_utc`
  - `metric_version`
  - `metrics_json`
  - `window_days` (fixed at 7 in v1)
  - `computed_at`, `updated_at`
- Indexes:
  - `idx_weekly_snapshots_repo_week(repository_id, week_start_utc)`

## Write Semantics

- Normalized signals: **upsert** using dedup key above.
- Weekly snapshots: **upsert** using snapshot key above.
- Upsert updates payload + timestamp fields and refreshes `updated_at`.

## Query Semantics

Single-repo optimized reads provided by database adapter methods:
- `list_normalized_signals_by_repository(repository_id, limit)`
  - returns newest-first by `occurred_at DESC` for ingestion history inspection.
- `list_weekly_metric_snapshots_by_repository(repository_id, limit)`
  - returns newest-first by `week_start_utc DESC`.
- `list_weekly_metric_snapshots_in_rolling_window(repository_id, window_start_utc, window_end_utc)`
  - inclusive range filter: `week_start_utc >= window_start_utc AND week_start_utc <= window_end_utc`.
  - ordered `week_start_utc DESC`.

Rolling-window semantics: caller supplies the time bounds; snapshots remain fixed 7-day bundles in v1 (`window_days = 7`).
