//! DORA Metrics Module
//!
//! Provides HTTP endpoints for querying weekly DORA metric snapshots and
//! a periodic background task that triggers refresh via `RefreshOrchestrator`.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use serde::Deserialize;
use tracing::error;

use crate::adapters::shared::database::Database;
use crate::adapters::shared::views::{DoraDashboardTemplate, HtmlTemplate, MetricItem, WeekEntry};
use crate::domain::metrics::WeeklyMetrics;

// ── State ────────────────────────────────────────────────────────────────────

/// Shared state for DORA metric handlers.
///
/// Holds a database handle for querying persisted snapshots.
/// The `RefreshOrchestrator` is owned by the background task (spawned separately).
#[derive(Clone)]
pub struct DoraAppState {
    pub db: Database,
}

// ── Query parameters ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct MetricsQuery {
    /// Optional week start date in ISO format (e.g. "2026-05-04").
    /// When absent, returns all available snapshots (up to 52 weeks).
    pub week: Option<String>,
}

// ── Dashboard Handler ────────────────────────────────────────────────────────

/// GET /{username}/{project}/{repo}/dora
///
/// Renders the DORA dashboard HTML page with metrics tables and charts.
/// Accepts an optional `?week=` query parameter to select a specific week.
async fn dashboard_handler(
    State(state): State<DoraAppState>,
    Path((username, project_name, repo_name)): Path<(String, String, String)>,
    Query(params): Query<MetricsQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Resolve user, project, repository from URL path
    let user = state
        .db
        .get_user_by_username(&username)
        .await
        .map_err(|e| {
            error!(error = %e, username = %username, "User not found");
            (
                StatusCode::NOT_FOUND,
                format!("User '{}' not found", username),
            )
        })?;

    let project = state
        .db
        .get_project(user.id, &project_name)
        .await
        .map_err(|e| {
            error!(error = %e, project = %project_name, "Project not found");
            (
                StatusCode::NOT_FOUND,
                format!("Project '{}' not found", project_name),
            )
        })?;

    let repo = state
        .db
        .get_repository(project.id, &repo_name)
        .await
        .map_err(|e| {
            error!(error = %e, repo = %repo_name, "Repository not found");
            (
                StatusCode::NOT_FOUND,
                format!("Repository '{}' not found", repo_name),
            )
        })?;

    // Load all available weekly snapshots.
    let all_snapshots = state
        .db
        .list_weekly_metric_snapshots_by_repository(repo.id, 52)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to query metric snapshots");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to query metric snapshots".to_string(),
            )
        })?;

    if all_snapshots.is_empty() {
        // No data yet — render empty state.
        return Ok(HtmlTemplate(DoraDashboardTemplate {
            username,
            project_name,
            repo_name,
            weeks: vec![],
            has_data: false,
            count_metrics: vec![],
            cycle_metrics: vec![],
            dora_metrics: vec![],
            median_metrics: vec![],
            chart_data_json: "{}".to_string(),
        }));
    }

    // ── Parse snapshots ──────────────────────────────────────────────────

    let mut parsed: Vec<(String, WeeklyMetrics)> = Vec::with_capacity(all_snapshots.len());
    for row in &all_snapshots {
        match serde_json::from_str::<crate::domain::metrics::WeeklyMetricSnapshot>(
            &row.metrics_json,
        ) {
            Ok(snapshot) => {
                parsed.push((row.week_start_utc.clone(), snapshot.metrics));
            }
            Err(e) => {
                error!(
                    error = %e,
                    week = %row.week_start_utc,
                    "Failed to parse weekly metric snapshot JSON"
                );
            }
        }
    }

    if parsed.is_empty() {
        // All rows had parse errors — show empty state.
        return Ok(HtmlTemplate(DoraDashboardTemplate {
            username,
            project_name,
            repo_name,
            weeks: vec![],
            has_data: false,
            count_metrics: vec![],
            cycle_metrics: vec![],
            dora_metrics: vec![],
            median_metrics: vec![],
            chart_data_json: "{}".to_string(),
        }));
    }

    // Sort by week ASC for charts (earliest first).
    parsed.sort_by(|a, b| a.0.cmp(&b.0));

    // Build week list (descending for dropdown — latest first).
    let weeks_desc: Vec<String> = parsed.iter().rev().map(|(w, _)| w.clone()).collect();

    // Determine selected week.
    let selected_week = params
        .week
        .as_ref()
        .and_then(|pw| weeks_desc.iter().find(|w| w.starts_with(pw)))
        .cloned()
        .unwrap_or_else(|| weeks_desc[0].clone());

    // Week entries for the dropdown.
    let weeks: Vec<WeekEntry> = weeks_desc
        .iter()
        .map(|w| WeekEntry {
            week_start: w.clone(),
            selected: *w == selected_week,
        })
        .collect();

    // Metrics for the selected week.
    let selected_metrics = parsed
        .iter()
        .find(|(w, _)| *w == selected_week)
        .map(|(_, m)| m);

    let (count_metrics, cycle_metrics, dora_metrics, median_metrics) =
        if let Some(m) = selected_metrics {
            build_metric_groups(m)
        } else {
            (vec![], vec![], vec![], vec![])
        };

    // Chart data across all weeks (pre-serialized JSON for Chart.js).
    let chart_data_json = build_chart_data(&parsed);

    Ok(HtmlTemplate(DoraDashboardTemplate {
        username,
        project_name,
        repo_name,
        weeks,
        has_data: true,
        count_metrics,
        cycle_metrics,
        dora_metrics,
        median_metrics,
        chart_data_json,
    }))
}

// ── Metrics Handler (JSON API) ───────────────────────────────────────────────

/// GET /{username}/{project}/{repo}/dora/metrics?week=2026-05-04
///
/// Returns a JSON array of weekly metric snapshot rows for the given repository.
/// If `week` is provided, only snapshots matching that week start are returned;
/// otherwise all available snapshots (up to 52 weeks) are returned.
async fn metrics_handler(
    State(state): State<DoraAppState>,
    Path((username, project_name, repo_name)): Path<(String, String, String)>,
    Query(params): Query<MetricsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Resolve user, project, repository from URL path
    let user = state
        .db
        .get_user_by_username(&username)
        .await
        .map_err(|e| {
            error!(error = %e, username = %username, "User not found");
            (
                StatusCode::NOT_FOUND,
                format!("User '{}' not found", username),
            )
        })?;

    let project = state
        .db
        .get_project(user.id, &project_name)
        .await
        .map_err(|e| {
            error!(error = %e, project = %project_name, "Project not found");
            (
                StatusCode::NOT_FOUND,
                format!("Project '{}' not found", project_name),
            )
        })?;

    let repo = state
        .db
        .get_repository(project.id, &repo_name)
        .await
        .map_err(|e| {
            error!(error = %e, repo = %repo_name, "Repository not found");
            (
                StatusCode::NOT_FOUND,
                format!("Repository '{}' not found", repo_name),
            )
        })?;

    let snapshots = if let Some(week) = &params.week {
        // Fetch all snapshots for the repo and filter by week prefix match.
        // We use a large limit and filter in Rust to avoid SQLite string comparison quirks.
        let all = state
            .db
            .list_weekly_metric_snapshots_by_repository(repo.id, 520)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to query metric snapshots");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to query metric snapshots".to_string(),
                )
            })?;

        all.into_iter()
            .filter(|row| row.week_start_utc.starts_with(week))
            .collect::<Vec<_>>()
    } else {
        state
            .db
            .list_weekly_metric_snapshots_by_repository(repo.id, 52)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to query metric snapshots");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to query metric snapshots".to_string(),
                )
            })?
    };

    Ok(Json(serde_json::json!(snapshots)))
}

// ── Router ───────────────────────────────────────────────────────────────────

/// Build the DORA router tree.
///
/// All DORA endpoints live under `/{username}/{project}/{repo}/dora/`.
pub fn routes() -> Router<DoraAppState> {
    Router::new()
        .route("/{username}/{project}/{repo}/dora", get(dashboard_handler))
        .route(
            "/{username}/{project}/{repo}/dora/metrics",
            get(metrics_handler),
        )
}

// ── Formatting helpers ───────────────────────────────────────────────────────

/// Format an optional duration in seconds to a human-readable string.
///
/// Returns "—" for `None`.
/// Examples: "3d 12h", "1h 30m", "45m 20s", "30s", "—"
fn format_duration(seconds: Option<i64>) -> String {
    match seconds {
        None => "—".to_string(),
        Some(s) if s >= 86_400 => {
            let days = s / 86_400;
            let hours = (s % 86_400) / 3_600;
            format!("{}d {}h", days, hours)
        }
        Some(s) if s >= 3_600 => {
            let hours = s / 3_600;
            let minutes = (s % 3_600) / 60;
            format!("{}h {}m", hours, minutes)
        }
        Some(s) if s >= 60 => {
            let minutes = s / 60;
            let secs = s % 60;
            format!("{}m {}s", minutes, secs)
        }
        Some(s) => format!("{}s", s),
    }
}

/// Format an optional change failure rate as a percentage string.
///
/// Returns "—" for `None`.
/// Example: Some(0.3333) → "33.3%", Some(0.0) → "0.0%"
fn format_cfr(rate: Option<f64>) -> String {
    match rate {
        None => "—".to_string(),
        Some(r) => format!("{:.1}%", r * 100.0),
    }
}

/// Build grouped metric display items from a `WeeklyMetrics` instance.
fn build_metric_groups(
    m: &WeeklyMetrics,
) -> (
    Vec<MetricItem>,
    Vec<MetricItem>,
    Vec<MetricItem>,
    Vec<MetricItem>,
) {
    let counts = vec![
        MetricItem {
            label: "PRs Opened".to_string(),
            value: m.pr_opened_count.to_string(),
        },
        MetricItem {
            label: "Merge Frequency".to_string(),
            value: m.merge_frequency_count.to_string(),
        },
        MetricItem {
            label: "Deployment Frequency".to_string(),
            value: m.deployment_frequency_count.to_string(),
        },
        MetricItem {
            label: "Commits".to_string(),
            value: m.commits_count.to_string(),
        },
        MetricItem {
            label: "Code Changes".to_string(),
            value: m.code_changes_count.to_string(),
        },
        MetricItem {
            label: "Total Reviews".to_string(),
            value: m.total_reviews_count.to_string(),
        },
        MetricItem {
            label: "Merged Without Review".to_string(),
            value: m.merged_without_review_count.to_string(),
        },
        MetricItem {
            label: "Handoffs".to_string(),
            value: m.handoffs_count.to_string(),
        },
    ];

    let cycles = vec![
        MetricItem {
            label: "Coding".to_string(),
            value: format_duration(m.cycle_coding_median_seconds),
        },
        MetricItem {
            label: "Pickup".to_string(),
            value: format_duration(m.cycle_pickup_median_seconds),
        },
        MetricItem {
            label: "Review".to_string(),
            value: format_duration(m.cycle_review_median_seconds),
        },
        MetricItem {
            label: "Deploy".to_string(),
            value: format_duration(m.cycle_deploy_median_seconds),
        },
    ];

    let dora = vec![
        MetricItem {
            label: "Deployment Frequency".to_string(),
            value: m.deployment_frequency_count.to_string(),
        },
        MetricItem {
            label: "Change Failure Rate".to_string(),
            value: format_cfr(m.change_failure_rate),
        },
        MetricItem {
            label: "MTTR".to_string(),
            value: format_duration(m.mttr_seconds),
        },
        MetricItem {
            label: "Lead Time for Changes".to_string(),
            value: format_duration(m.lead_time_for_changes_median_seconds),
        },
    ];

    let medians = vec![
        MetricItem {
            label: "PR Size".to_string(),
            value: m.pr_size_median.map_or("—".to_string(), |v| v.to_string()),
        },
        MetricItem {
            label: "Review Depth".to_string(),
            value: m
                .review_depth_median
                .map_or("—".to_string(), |v| v.to_string()),
        },
        MetricItem {
            label: "Review Time".to_string(),
            value: format_duration(m.review_time_median_seconds),
        },
        MetricItem {
            label: "PR Pickup Time".to_string(),
            value: format_duration(m.pr_pickup_time_median_seconds),
        },
        MetricItem {
            label: "Time to Merge".to_string(),
            value: format_duration(m.time_to_merge_median_seconds),
        },
        MetricItem {
            label: "Deployment Time".to_string(),
            value: format_duration(m.deployment_time_median_seconds),
        },
    ];

    (counts, cycles, dora, medians)
}

/// Build Chart.js-compatible JSON data from all parsed weekly snapshots.
///
/// Returns a pre-serialized JSON string with grouped datasets across all weeks.
fn build_chart_data(parsed: &[(String, WeeklyMetrics)]) -> String {
    let labels: Vec<&str> = parsed.iter().map(|(w, _)| w.as_str()).collect();

    // ── Counts ───────────────────────────────────────────────────────────
    let counts_datasets = serde_json::json!({
        "PRs Opened": parsed.iter().map(|(_, m)| m.pr_opened_count).collect::<Vec<_>>(),
        "Merged": parsed.iter().map(|(_, m)| m.merge_frequency_count).collect::<Vec<_>>(),
        "Deployments": parsed.iter().map(|(_, m)| m.deployment_frequency_count).collect::<Vec<_>>(),
        "Commits": parsed.iter().map(|(_, m)| m.commits_count).collect::<Vec<_>>(),
        "Code Changes": parsed.iter().map(|(_, m)| m.code_changes_count).collect::<Vec<_>>(),
        "Reviews": parsed.iter().map(|(_, m)| m.total_reviews_count).collect::<Vec<_>>(),
    });

    // ── Durations (in seconds) ───────────────────────────────────────────
    let durations_datasets = serde_json::json!({
        "Review Time": parsed.iter().map(|(_, m)| m.review_time_median_seconds).collect::<Vec<_>>(),
        "Pickup Time": parsed.iter().map(|(_, m)| m.pr_pickup_time_median_seconds).collect::<Vec<_>>(),
        "Time to Merge": parsed.iter().map(|(_, m)| m.time_to_merge_median_seconds).collect::<Vec<_>>(),
        "Deploy Time": parsed.iter().map(|(_, m)| m.deployment_time_median_seconds).collect::<Vec<_>>(),
        "Lead Time": parsed.iter().map(|(_, m)| m.lead_time_for_changes_median_seconds).collect::<Vec<_>>(),
        "MTTR": parsed.iter().map(|(_, m)| m.mttr_seconds).collect::<Vec<_>>(),
    });

    // ── Cycle stages ────────────────────────────────────────────────────
    let cycles_datasets = serde_json::json!({
        "Coding": parsed.iter().map(|(_, m)| m.cycle_coding_median_seconds).collect::<Vec<_>>(),
        "Pickup": parsed.iter().map(|(_, m)| m.cycle_pickup_median_seconds).collect::<Vec<_>>(),
        "Review": parsed.iter().map(|(_, m)| m.cycle_review_median_seconds).collect::<Vec<_>>(),
        "Deploy": parsed.iter().map(|(_, m)| m.cycle_deploy_median_seconds).collect::<Vec<_>>(),
    });

    // ── CFR ─────────────────────────────────────────────────────────────
    let cfr_datasets = serde_json::json!({
        "Change Failure Rate": parsed.iter().map(|(_, m)| m.change_failure_rate).collect::<Vec<_>>(),
    });

    let chart = serde_json::json!({
        "counts": {
            "labels": labels,
            "datasets": counts_datasets,
        },
        "durations": {
            "labels": labels,
            "datasets": durations_datasets,
        },
        "cycles": {
            "labels": labels,
            "datasets": cycles_datasets,
        },
        "cfr": {
            "labels": labels,
            "datasets": cfr_datasets,
        },
    });

    chart.to_string()
}
