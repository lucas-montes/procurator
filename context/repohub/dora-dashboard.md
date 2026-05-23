# Repohub DORA Dashboard (T09)

Read-only HTML dashboard rendering weekly DORA/productivity metrics for one repository.

Defined in `repohub/src/application/dora/mod.rs` (handler + formatting helpers) and `repohub/templates/dora/dashboard.html` (Askama template).

## Route

```
GET /{username}/{project}/{repo}/dora
```

Optional query parameter `?week=<ISO date>` selects a specific week (defaults to most recent).

**Behavior:**
- Resolves user → project → repository from URL path (404 if not found).
- Loads up to 52 weekly snapshots via `list_weekly_metric_snapshots_by_repository`.
- If no snapshots exist or all rows fail JSON parse, renders an empty state with icon and message "No metrics yet — waiting for first refresh".
- Sorts snapshots ascending for chart order; displays week dropdown descending (latest first).
- Selected week is determined by `?week=` param (prefix match) or falls back to the most recent snapshot.
- Pre-serializes Chart.js JSON (`chart_data_json`) covering all available weeks.

## Template Structure

The Askama template (`dora/dashboard.html`) renders into `base.html`:

| Section | Content |
|---|---|
| Breadcrumb | Users → username → project → repo → DORA |
| Week picker | `<select>` dropdown with all available week dates, client-side navigation via `navigateWeek()` |
| Counts | PRs Opened, Merge Frequency, Deployment Frequency, Commits, Code Changes, Total Reviews, Merged Without Review, Handoffs |
| Cycle Stages (Median) | Coding, Pickup, Review, Deploy |
| DORA Rates | Deployment Frequency, Change Failure Rate, MTTR, Lead Time for Changes |
| Medians | PR Size, Review Depth, Review Time, PR Pickup Time, Time to Merge, Deployment Time |
| Trend Charts (4) | Counts (bar), Median Durations (line), Cycle Stages (line), DORA Rates (line) |

## Chart.js Integration

- Loaded from CDN: `https://cdn.jsdelivr.net/npm/chart.js@4.4.7/dist/chart.umd.min.js`
- Data injected as `chart_data_json` — pre-serialized JSON with labels and grouped datasets across all weeks.
- Four charts rendered on `DOMContentLoaded`:
  - **countsChart** — bar chart (PRs Opened, Merged, Deployments, Commits, Code Changes, Reviews)
  - **durationsChart** — line chart (Review Time, Pickup Time, Time to Merge, Deploy Time, Lead Time, MTTR) with `formatSeconds` Y-axis ticks
  - **cyclesChart** — line chart (Coding, Pickup, Review, Deploy) with `formatSeconds` Y-axis ticks
  - **cfrChart** — line chart (Change Failure Rate as raw float)
- Color palette of 8 colors cycled per dataset.
- Y-axis begins at zero; X-axis labels rotated 45° max.

## JS Helper Functions

| Function | Purpose |
|---|---|
| `navigateWeek(week)` | Sets `?week=` search param and navigates |
| `buildChart(canvasId, type, labels, datasets, unit)` | Creates a Chart.js instance; `unit='s'` enables `formatSeconds` ticks |
| `formatSeconds(s)` | Converts seconds → `Xd`, `Xh`, `Xm`, `Xs` or empty string for null |

## Rust Formatting Helpers

Defined in `dora/mod.rs` (not exported — module-private):

| Function | Signature | Behavior |
|---|---|---|
| `format_duration` | `fn(Option<i64>) -> String` | `None` → `"—"`; `≥86400` → `"Xd Xh"`; `≥3600` → `"Xh Xm"`; `≥60` → `"Xm Xs"`; else `"Xs"` |
| `format_cfr` | `fn(Option<f64>) -> String` | `None` → `"—"`; `Some(r)` → `"{:.1}%"` (multiplied by 100) |

## Build Metric Groups

`build_metric_groups(m: &WeeklyMetrics)` partitions metrics into four `Vec<MetricItem>` buckets using `format_duration`/`format_cfr` for display-friendly strings.

## Navigation

The repository page (`templates/repository.html`) includes a card linking to the DORA dashboard:
```
📊 DORA Dashboard — View DORA and productivity metrics
/href to /{username}/{project}/{repo}/dora
```

## Differences from JSON API

| Aspect | Dashboard (`/dora`) | Metrics API (`/dora/metrics`) |
|---|---|---|
| Response | HTML (Askama template) | JSON (raw `WeeklyMetricSnapshotRow` array) |
| Week filtering | Single week selected via dropdown | Optional week filter, returns raw rows |
| Charts | 4 Chart.js trend charts over all weeks | None (JSON only) |
| Purpose | Human-readable display | Machine-consumable data |

## Test Coverage

No dedicated tests for the dashboard handler (v1). Manual render verification via `?week=` parameter navigation. The template is validated at compile time by Askama.

See also: [dora-api.md](dora-api.md), [weekly-metrics-engine.md](weekly-metrics-engine.md), [overview.md](../overview.md), [context-map.md](../context-map.md)
