use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::signals::{
    NormalizedCommit, NormalizedDeployment, NormalizedIssue, NormalizedPullRequest,
    NormalizedReview,
};

pub const WEEKLY_WINDOW_DAYS: i64 = 7;

#[derive(Debug, Clone)]
pub struct WeeklyMetricInput<'a> {
    pub repository_id: i64,
    pub week_start_utc: DateTime<Utc>,
    pub metric_version: &'a str,
    pub incident_label_patterns: &'a [String],
    pub pull_requests: &'a [NormalizedPullRequest],
    pub reviews: &'a [NormalizedReview],
    pub commits: &'a [NormalizedCommit],
    pub deployments: &'a [NormalizedDeployment],
    pub issues: &'a [NormalizedIssue],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeeklyMetricSnapshot {
    pub repository_id: i64,
    pub week_start_utc: String,
    pub window_days: i64,
    pub metric_version: String,
    pub metrics: WeeklyMetrics,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeeklyMetrics {
    pub pr_opened_count: i64,
    pub merge_frequency_count: i64,
    pub deployment_frequency_count: i64,
    pub commits_count: i64,
    pub code_changes_count: i64,
    pub total_reviews_count: i64,
    pub merged_without_review_count: i64,
    pub handoffs_count: i64,

    pub pr_size_median: Option<i64>,
    pub review_depth_median: Option<i64>,

    pub review_time_median_seconds: Option<i64>,
    pub pr_pickup_time_median_seconds: Option<i64>,
    pub time_to_merge_median_seconds: Option<i64>,
    pub deployment_time_median_seconds: Option<i64>,
    pub lead_time_for_changes_median_seconds: Option<i64>,

    pub cycle_coding_median_seconds: Option<i64>,
    pub cycle_pickup_median_seconds: Option<i64>,
    pub cycle_review_median_seconds: Option<i64>,
    pub cycle_deploy_median_seconds: Option<i64>,

    pub change_failure_rate: Option<f64>,
    pub mttr_seconds: Option<i64>,
}

pub struct WeeklyMetricEngine;

impl WeeklyMetricEngine {
    pub fn compute(input: WeeklyMetricInput<'_>) -> WeeklyMetricSnapshot {
        let window_start = input.week_start_utc;
        let window_end = window_start + Duration::days(WEEKLY_WINDOW_DAYS);

        let mut prs: Vec<&NormalizedPullRequest> = input
            .pull_requests
            .iter()
            .filter(|pr| pr.repository_id == input.repository_id)
            .collect();
        prs.sort_by_key(|pr| (pr.opened_at(), pr.id));

        let pr_ids: BTreeSet<i64> = prs.iter().map(|pr| pr.id).collect();

        let mut reviews_by_pr: BTreeMap<i64, Vec<&NormalizedReview>> = BTreeMap::new();
        for review in input
            .reviews
            .iter()
            .filter(|review| pr_ids.contains(&review.pull_request_id))
        {
            reviews_by_pr
                .entry(review.pull_request_id)
                .or_default()
                .push(review);
        }
        for reviews in reviews_by_pr.values_mut() {
            reviews.sort_by_key(|review| (review.submitted_timestamp(), review.id));
        }

        let mut commits: Vec<&NormalizedCommit> = input
            .commits
            .iter()
            .filter(|commit| commit.repository_id == input.repository_id)
            .collect();
        commits.sort_by_key(|commit| (commit.committed_timestamp(), commit.sha.clone()));

        let mut commits_by_sha: BTreeMap<String, Vec<&NormalizedCommit>> = BTreeMap::new();
        for commit in &commits {
            commits_by_sha
                .entry(commit.sha.clone())
                .or_default()
                .push(*commit);
        }

        let mut deployments: Vec<&NormalizedDeployment> = input
            .deployments
            .iter()
            .filter(|deployment| {
                deployment.repository_id == input.repository_id && deployment.is_production()
            })
            .collect();
        deployments.sort_by_key(|deployment| (deployment.deployed_at(), deployment.id));

        let mut deployments_by_sha: BTreeMap<String, Vec<&NormalizedDeployment>> = BTreeMap::new();
        for deployment in &deployments {
            deployments_by_sha
                .entry(deployment.sha.clone())
                .or_default()
                .push(*deployment);
        }

        let mut issues: Vec<&NormalizedIssue> = input
            .issues
            .iter()
            .filter(|issue| issue.repository_id == input.repository_id)
            .collect();
        issues.sort_by_key(|issue| (issue.opened_at(), issue.id));

        let prs_opened_in_window: Vec<&NormalizedPullRequest> = prs
            .iter()
            .copied()
            .filter(|pr| in_window(pr.opened_at(), window_start, window_end))
            .collect();

        let prs_merged_in_window: Vec<&NormalizedPullRequest> = prs
            .iter()
            .copied()
            .filter(|pr| {
                pr.merged_timestamp()
                    .is_some_and(|merged_at| in_window(merged_at, window_start, window_end))
            })
            .collect();

        let reviews_in_window: Vec<&NormalizedReview> = input
            .reviews
            .iter()
            .filter(|review| pr_ids.contains(&review.pull_request_id))
            .filter(|review| in_window(review.submitted_timestamp(), window_start, window_end))
            .collect();

        let commits_in_window: Vec<&NormalizedCommit> = commits
            .iter()
            .copied()
            .filter(|commit| in_window(commit.committed_timestamp(), window_start, window_end))
            .collect();

        let deployments_in_window: Vec<&NormalizedDeployment> = deployments
            .iter()
            .copied()
            .filter(|deployment| in_window(deployment.deployed_at(), window_start, window_end))
            .collect();

        let pr_size_median = median_i64(
            prs_opened_in_window
                .iter()
                .map(|pr| i64::from(pr.size()))
                .collect(),
        );

        let review_depth_median = median_i64(
            prs_opened_in_window
                .iter()
                .map(|pr| {
                    reviews_by_pr
                        .get(&pr.id)
                        .map_or(0_i64, |reviews| reviews.len() as i64)
                })
                .collect(),
        );

        let review_time_durations =
            review_time_or_pickup_durations(&prs, &reviews_by_pr, window_start, window_end);

        let time_to_merge_durations = time_to_merge_durations(&prs, window_start, window_end);

        let deployment_time_stage_durations =
            deployment_time_durations(&prs, &deployments_by_sha, window_start, window_end);

        let lead_time_durations = lead_time_for_change_durations(
            &prs,
            &commits_by_sha,
            &deployments_by_sha,
            window_start,
            window_end,
        );

        let cycle_coding_durations =
            cycle_coding_durations(&prs, &commits_by_sha, window_start, window_end);

        let cycle_pickup_durations =
            review_time_or_pickup_durations(&prs, &reviews_by_pr, window_start, window_end);

        let cycle_review_durations =
            cycle_review_durations(&prs, &reviews_by_pr, window_start, window_end);

        let cycle_deploy_durations =
            deployment_time_durations(&prs, &deployments_by_sha, window_start, window_end);

        let merged_without_review_count = prs_merged_in_window
            .iter()
            .filter(|pr| {
                reviews_by_pr
                    .get(&pr.id)
                    .is_none_or(|reviews| reviews.is_empty())
            })
            .count() as i64;

        let handoffs_count = handoffs_count(&reviews_by_pr, window_start, window_end);

        let (change_failure_rate, mttr_seconds) = change_failure_rate_and_mttr(
            input.repository_id,
            &deployments,
            &issues,
            input.incident_label_patterns,
            window_start,
            window_end,
        );

        WeeklyMetricSnapshot {
            repository_id: input.repository_id,
            week_start_utc: window_start.to_rfc3339(),
            window_days: WEEKLY_WINDOW_DAYS,
            metric_version: input.metric_version.to_string(),
            metrics: WeeklyMetrics {
                pr_opened_count: prs_opened_in_window.len() as i64,
                merge_frequency_count: prs_merged_in_window.len() as i64,
                deployment_frequency_count: deployments_in_window.len() as i64,
                commits_count: commits_in_window.len() as i64,
                code_changes_count: commits_in_window
                    .iter()
                    .map(|commit| i64::from(commit.size()))
                    .sum(),
                total_reviews_count: reviews_in_window.len() as i64,
                merged_without_review_count,
                handoffs_count,

                pr_size_median,
                review_depth_median,

                review_time_median_seconds: median_i64(review_time_durations.clone()),
                pr_pickup_time_median_seconds: median_i64(review_time_durations),
                time_to_merge_median_seconds: median_i64(time_to_merge_durations),
                deployment_time_median_seconds: median_i64(deployment_time_stage_durations),
                lead_time_for_changes_median_seconds: median_i64(lead_time_durations),

                cycle_coding_median_seconds: median_i64(cycle_coding_durations),
                cycle_pickup_median_seconds: median_i64(cycle_pickup_durations),
                cycle_review_median_seconds: median_i64(cycle_review_durations),
                cycle_deploy_median_seconds: median_i64(cycle_deploy_durations),

                change_failure_rate,
                mttr_seconds,
            },
        }
    }

    pub fn serialize_payload(snapshot: &WeeklyMetricSnapshot) -> Result<String, serde_json::Error> {
        serde_json::to_string(snapshot)
    }
}

fn in_window(timestamp: DateTime<Utc>, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
    timestamp >= start && timestamp < end
}

fn seconds_between(start: DateTime<Utc>, end: DateTime<Utc>) -> i64 {
    (end - start).num_seconds()
}

fn median_i64(mut values: Vec<i64>) -> Option<i64> {
    if values.is_empty() {
        return None;
    }

    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[middle])
    } else {
        Some((values[middle - 1] + values[middle]) / 2)
    }
}

fn first_review_for_pr<'a>(
    pr_id: i64,
    reviews_by_pr: &'a BTreeMap<i64, Vec<&'a NormalizedReview>>,
) -> Option<&'a NormalizedReview> {
    reviews_by_pr
        .get(&pr_id)
        .and_then(|reviews| reviews.first().copied())
}

fn first_commit_for_pr<'a>(
    pr: &NormalizedPullRequest,
    commits_by_sha: &'a BTreeMap<String, Vec<&'a NormalizedCommit>>,
) -> Option<&'a NormalizedCommit> {
    let mut candidates: Vec<&NormalizedCommit> = Vec::new();

    if let Some(commits) = commits_by_sha.get(&pr.head_sha) {
        candidates.extend(commits.iter().copied());
    }
    if let Some(merge_sha) = &pr.merge_commit_sha {
        if let Some(commits) = commits_by_sha.get(merge_sha) {
            candidates.extend(commits.iter().copied());
        }
    }

    candidates
        .into_iter()
        .min_by_key(|commit| (commit.coding_started_at(), commit.sha.clone()))
}

fn first_deployment_for_pr(
    pr: &NormalizedPullRequest,
    deployments_by_sha: &BTreeMap<String, Vec<&NormalizedDeployment>>,
    merged_at: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let mut candidates: Vec<&NormalizedDeployment> = Vec::new();

    if let Some(merge_sha) = &pr.merge_commit_sha {
        if let Some(deployments) = deployments_by_sha.get(merge_sha) {
            candidates.extend(deployments.iter().copied());
        }
    }

    if let Some(deployments) = deployments_by_sha.get(&pr.head_sha) {
        candidates.extend(deployments.iter().copied());
    }

    candidates
        .into_iter()
        .filter(|deployment| deployment.deployed_at() >= merged_at)
        .min_by_key(|deployment| (deployment.deployed_at(), deployment.id))
        .map(|deployment| deployment.deployed_at())
}

fn review_time_or_pickup_durations(
    prs: &[&NormalizedPullRequest],
    reviews_by_pr: &BTreeMap<i64, Vec<&NormalizedReview>>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Vec<i64> {
    prs.iter()
        .filter_map(|pr| {
            let first_review = first_review_for_pr(pr.id, reviews_by_pr)?;
            let first_review_at = first_review.submitted_timestamp();
            if !in_window(first_review_at, window_start, window_end) {
                return None;
            }
            Some(seconds_between(pr.opened_at(), first_review_at))
        })
        .collect()
}

fn time_to_merge_durations(
    prs: &[&NormalizedPullRequest],
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Vec<i64> {
    prs.iter()
        .filter_map(|pr| {
            let merged_at = pr.merged_timestamp()?;
            if !in_window(merged_at, window_start, window_end) {
                return None;
            }
            Some(seconds_between(pr.opened_at(), merged_at))
        })
        .collect()
}

fn deployment_time_durations(
    prs: &[&NormalizedPullRequest],
    deployments_by_sha: &BTreeMap<String, Vec<&NormalizedDeployment>>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Vec<i64> {
    prs.iter()
        .filter_map(|pr| {
            let merged_at = pr.merged_timestamp()?;
            let deployed_at = first_deployment_for_pr(pr, deployments_by_sha, merged_at)?;
            if !in_window(deployed_at, window_start, window_end) {
                return None;
            }
            Some(seconds_between(merged_at, deployed_at))
        })
        .collect()
}

fn lead_time_for_change_durations(
    prs: &[&NormalizedPullRequest],
    commits_by_sha: &BTreeMap<String, Vec<&NormalizedCommit>>,
    deployments_by_sha: &BTreeMap<String, Vec<&NormalizedDeployment>>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Vec<i64> {
    prs.iter()
        .filter_map(|pr| {
            let merged_at = pr.merged_timestamp()?;
            let deployed_at = first_deployment_for_pr(pr, deployments_by_sha, merged_at)?;
            if !in_window(deployed_at, window_start, window_end) {
                return None;
            }

            let first_commit = first_commit_for_pr(pr, commits_by_sha)?;
            Some(seconds_between(
                first_commit.coding_started_at(),
                deployed_at,
            ))
        })
        .collect()
}

fn cycle_coding_durations(
    prs: &[&NormalizedPullRequest],
    commits_by_sha: &BTreeMap<String, Vec<&NormalizedCommit>>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Vec<i64> {
    prs.iter()
        .filter(|pr| in_window(pr.opened_at(), window_start, window_end))
        .filter_map(|pr| {
            let first_commit = first_commit_for_pr(pr, commits_by_sha)?;
            Some(seconds_between(
                first_commit.coding_started_at(),
                pr.opened_at(),
            ))
        })
        .collect()
}

fn cycle_review_durations(
    prs: &[&NormalizedPullRequest],
    reviews_by_pr: &BTreeMap<i64, Vec<&NormalizedReview>>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Vec<i64> {
    prs.iter()
        .filter_map(|pr| {
            let merged_at = pr.merged_timestamp()?;
            if !in_window(merged_at, window_start, window_end) {
                return None;
            }
            let first_review = first_review_for_pr(pr.id, reviews_by_pr)?;
            Some(seconds_between(
                first_review.submitted_timestamp(),
                merged_at,
            ))
        })
        .collect()
}

fn handoffs_count(
    reviews_by_pr: &BTreeMap<i64, Vec<&NormalizedReview>>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> i64 {
    reviews_by_pr
        .values()
        .map(|reviews| {
            let in_window_reviews: Vec<&NormalizedReview> = reviews
                .iter()
                .copied()
                .filter(|review| in_window(review.submitted_timestamp(), window_start, window_end))
                .collect();

            in_window_reviews
                .windows(2)
                .filter(|window| window[0].user_id != window[1].user_id)
                .count() as i64
        })
        .sum()
}

#[derive(Debug, Clone)]
struct FailurePoint {
    occurred_at: DateTime<Utc>,
    source_key: String,
    is_recovery: bool,
}

fn change_failure_rate_and_mttr(
    repository_id: i64,
    deployments: &[&NormalizedDeployment],
    issues: &[&NormalizedIssue],
    incident_label_patterns: &[String],
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> (Option<f64>, Option<i64>) {
    let mut points: Vec<FailurePoint> = deployments
        .iter()
        .filter(|deployment| deployment.repository_id == repository_id)
        .map(|deployment| FailurePoint {
            occurred_at: deployment.deployed_at(),
            source_key: format!("deployment:{}", deployment.id),
            is_recovery: deployment.is_success(),
        })
        .collect();

    points.extend(
        issues
            .iter()
            .filter(|issue| issue.repository_id == repository_id)
            .filter_map(|issue| {
                issue
                    .to_failure_signal(incident_label_patterns)
                    .map(|signal| FailurePoint {
                        occurred_at: signal.occurred_at,
                        source_key: format!("issue:{}", issue.id),
                        is_recovery: signal.is_recovery,
                    })
            }),
    );

    points.sort_by_key(|point| (point.occurred_at, point.source_key.clone()));

    // Mixed-model failure episodes: first failure after recovery/clean state.
    let mut failure_indices: Vec<usize> = Vec::new();
    let mut in_failure = false;

    for (index, point) in points.iter().enumerate() {
        if point.is_recovery {
            in_failure = false;
            continue;
        }

        if !in_failure {
            in_failure = true;
            failure_indices.push(index);
        }
    }

    let failures_in_window: Vec<usize> = failure_indices
        .iter()
        .copied()
        .filter(|index| in_window(points[*index].occurred_at, window_start, window_end))
        .collect();

    let deployment_count = deployments
        .iter()
        .filter(|deployment| in_window(deployment.deployed_at(), window_start, window_end))
        .count() as i64;

    let change_failure_rate = if deployment_count == 0 {
        None
    } else {
        Some((failures_in_window.len() as f64 / deployment_count as f64).clamp(0.0, 1.0))
    };

    // Nearest-forward recovery matching in deterministic order.
    let mut mttr_durations = Vec::new();
    let mut next_recovery_search_start = 0_usize;

    for failure_index in failures_in_window {
        let recovery = points
            .iter()
            .enumerate()
            .skip(next_recovery_search_start.max(failure_index + 1))
            .find(|(_, point)| point.is_recovery);

        if let Some((recovery_index, recovery)) = recovery {
            next_recovery_search_start = recovery_index + 1;
            mttr_durations.push(seconds_between(
                points[failure_index].occurred_at,
                recovery.occurred_at,
            ));
        }
    }

    (change_failure_rate, median_i64(mttr_durations))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    fn pr(
        id: i64,
        number: i32,
        opened: &str,
        merged: Option<&str>,
        additions: u32,
        deletions: u32,
        head_sha: &str,
    ) -> NormalizedPullRequest {
        NormalizedPullRequest {
            id,
            repository_id: 42,
            number,
            title: format!("PR {number}"),
            author_id: 900 + id,
            state: if merged.is_some() { "closed" } else { "open" }.to_string(),
            created_at: ts(opened),
            updated_at: ts(opened),
            closed_at: merged.map(ts),
            merged_at: merged.map(ts),
            additions,
            deletions,
            changed_files: 2,
            head_sha: head_sha.to_string(),
            head_ref: "feature/x".to_string(),
            base_sha: "base".to_string(),
            base_ref: "main".to_string(),
            merge_commit_sha: Some(format!("merge-{head_sha}")),
            draft: false,
            author_association: None,
        }
    }

    fn review(id: i64, pr_id: i64, user_id: i64, submitted_at: &str) -> NormalizedReview {
        NormalizedReview {
            id,
            pull_request_id: pr_id,
            user_id,
            state: "approved".to_string(),
            submitted_at: ts(submitted_at),
            body: None,
            commit_id: None,
        }
    }

    fn commit(
        sha: &str,
        authored_at: &str,
        committed_at: &str,
        additions: u32,
        deletions: u32,
    ) -> NormalizedCommit {
        NormalizedCommit {
            sha: sha.to_string(),
            repository_id: 42,
            author_id: 1,
            committer_id: 1,
            message: "commit".to_string(),
            authored_at: ts(authored_at),
            committed_at: ts(committed_at),
            additions,
            deletions,
            total: additions + deletions,
        }
    }

    fn deployment(id: i64, sha: &str, state: &str, at: &str) -> NormalizedDeployment {
        NormalizedDeployment {
            id,
            repository_id: 42,
            sha: sha.to_string(),
            ref_field: "refs/heads/main".to_string(),
            task: "deploy".to_string(),
            payload: None,
            environment: "production".to_string(),
            state: state.to_string(),
            created_at: ts(at),
            updated_at: ts(at),
            creator_id: Some(1),
            description: None,
            is_production: true,
        }
    }

    fn issue(
        id: i64,
        state: &str,
        opened: &str,
        closed: Option<&str>,
        labels: &[&str],
    ) -> NormalizedIssue {
        NormalizedIssue {
            id,
            repository_id: 42,
            number: id as i32,
            title: "incident".to_string(),
            author_id: 1,
            state: state.to_string(),
            created_at: ts(opened),
            updated_at: ts(opened),
            closed_at: closed.map(ts),
            labels: labels.iter().map(|label| label.to_string()).collect(),
            is_pull_request: false,
            assignee_id: None,
            milestone_id: None,
        }
    }

    #[test]
    fn computes_all_weekly_metrics_from_normalized_signals() {
        let week_start = ts("2026-05-04T00:00:00Z");
        let patterns = vec![".*incident.*".to_string()];

        let prs = vec![
            pr(
                1,
                10,
                "2026-05-04T00:00:00Z",
                Some("2026-05-05T00:00:00Z"),
                10,
                2,
                "sha-1",
            ),
            pr(
                2,
                11,
                "2026-05-06T00:00:00Z",
                Some("2026-05-06T10:00:00Z"),
                20,
                10,
                "sha-2",
            ),
            pr(3, 12, "2026-05-10T23:59:59Z", None, 5, 5, "sha-3"),
            pr(4, 13, "2026-05-11T00:00:00Z", None, 99, 1, "sha-4"),
        ];

        let reviews = vec![
            review(1, 1, 100, "2026-05-04T02:00:00Z"),
            review(2, 1, 101, "2026-05-04T03:00:00Z"),
            review(3, 2, 200, "2026-05-06T01:00:00Z"),
            review(4, 3, 300, "2026-05-10T23:59:59Z"),
            review(5, 3, 301, "2026-05-11T00:00:00Z"),
        ];

        let commits = vec![
            commit(
                "sha-1",
                "2026-05-03T20:00:00Z",
                "2026-05-04T01:00:00Z",
                4,
                2,
            ),
            commit(
                "sha-2",
                "2026-05-05T20:00:00Z",
                "2026-05-06T02:00:00Z",
                2,
                1,
            ),
            commit(
                "sha-3",
                "2026-05-10T20:00:00Z",
                "2026-05-10T20:10:00Z",
                3,
                2,
            ),
            commit(
                "sha-4",
                "2026-05-11T00:00:00Z",
                "2026-05-11T00:00:00Z",
                10,
                10,
            ),
            commit(
                "merge-sha-1",
                "2026-05-05T00:00:00Z",
                "2026-05-05T00:00:00Z",
                1,
                0,
            ),
            commit(
                "merge-sha-2",
                "2026-05-06T10:00:00Z",
                "2026-05-06T10:00:00Z",
                1,
                0,
            ),
        ];

        let deployments = vec![
            deployment(1, "merge-sha-1", "success", "2026-05-05T03:00:00Z"),
            deployment(2, "merge-sha-2", "failure", "2026-05-06T11:00:00Z"),
            deployment(3, "merge-sha-2", "success", "2026-05-06T12:00:00Z"),
        ];

        let issues = vec![
            issue(1, "open", "2026-05-06T11:00:00Z", None, &["incident"]),
            issue(
                2,
                "closed",
                "2026-05-06T11:00:00Z",
                Some("2026-05-06T13:00:00Z"),
                &["incident"],
            ),
        ];

        let snapshot = WeeklyMetricEngine::compute(WeeklyMetricInput {
            repository_id: 42,
            week_start_utc: week_start,
            metric_version: "v1",
            incident_label_patterns: &patterns,
            pull_requests: &prs,
            reviews: &reviews,
            commits: &commits,
            deployments: &deployments,
            issues: &issues,
        });

        assert_eq!(snapshot.metrics.pr_opened_count, 3);
        assert_eq!(snapshot.metrics.merge_frequency_count, 2);
        assert_eq!(snapshot.metrics.deployment_frequency_count, 3);
        assert_eq!(snapshot.metrics.commits_count, 5);
        assert_eq!(snapshot.metrics.code_changes_count, 16);
        assert_eq!(snapshot.metrics.total_reviews_count, 4);
        assert_eq!(snapshot.metrics.merged_without_review_count, 0);
        assert_eq!(snapshot.metrics.handoffs_count, 1);

        assert_eq!(snapshot.metrics.pr_size_median, Some(12));
        assert_eq!(snapshot.metrics.review_depth_median, Some(2));

        assert_eq!(snapshot.metrics.review_time_median_seconds, Some(3600));
        assert_eq!(snapshot.metrics.pr_pickup_time_median_seconds, Some(3600));
        assert_eq!(snapshot.metrics.time_to_merge_median_seconds, Some(61_200));
        assert_eq!(snapshot.metrics.deployment_time_median_seconds, Some(7_200));
        assert_eq!(
            snapshot.metrics.lead_time_for_changes_median_seconds,
            Some(82_800)
        );

        assert_eq!(snapshot.metrics.cycle_coding_median_seconds, Some(14_400));
        assert_eq!(snapshot.metrics.cycle_pickup_median_seconds, Some(3600));
        assert_eq!(snapshot.metrics.cycle_review_median_seconds, Some(55_800));
        assert_eq!(snapshot.metrics.cycle_deploy_median_seconds, Some(7_200));

        assert_eq!(snapshot.metrics.change_failure_rate, Some(1.0 / 3.0));
        assert_eq!(snapshot.metrics.mttr_seconds, Some(3_600));
    }

    #[test]
    fn respects_event_timestamp_window_boundaries_and_nullables() {
        let week_start = ts("2026-05-04T00:00:00Z");
        let patterns = vec![".*incident.*".to_string()];

        let prs = vec![
            pr(
                1,
                10,
                "2026-05-03T23:59:59Z",
                Some("2026-05-11T00:00:00Z"),
                1,
                1,
                "sha-1",
            ),
            pr(2, 11, "2026-05-04T00:00:00Z", None, 2, 2, "sha-2"),
            pr(3, 12, "2026-05-10T23:59:59Z", None, 3, 3, "sha-3"),
        ];

        let reviews = vec![
            review(1, 2, 1, "2026-05-10T23:59:59Z"),
            review(2, 2, 2, "2026-05-11T00:00:00Z"),
        ];

        let commits = vec![commit(
            "sha-2",
            "2026-05-03T20:00:00Z",
            "2026-05-11T00:00:00Z",
            10,
            10,
        )];

        let snapshot = WeeklyMetricEngine::compute(WeeklyMetricInput {
            repository_id: 42,
            week_start_utc: week_start,
            metric_version: "v1",
            incident_label_patterns: &patterns,
            pull_requests: &prs,
            reviews: &reviews,
            commits: &commits,
            deployments: &[],
            issues: &[],
        });

        assert_eq!(snapshot.metrics.pr_opened_count, 2);
        assert_eq!(snapshot.metrics.merge_frequency_count, 0);
        assert_eq!(snapshot.metrics.total_reviews_count, 1);
        assert_eq!(snapshot.metrics.commits_count, 0);

        assert_eq!(snapshot.metrics.deployment_frequency_count, 0);
        assert_eq!(snapshot.metrics.change_failure_rate, None);
        assert_eq!(snapshot.metrics.mttr_seconds, None);
        assert_eq!(snapshot.metrics.time_to_merge_median_seconds, None);
        assert_eq!(snapshot.metrics.deployment_time_median_seconds, None);
        assert_eq!(snapshot.metrics.lead_time_for_changes_median_seconds, None);
    }

    #[test]
    fn median_even_and_empty_behavior_matches_contract() {
        assert_eq!(median_i64(vec![]), None);
        assert_eq!(median_i64(vec![10, 20]), Some(15));
        assert_eq!(median_i64(vec![1, 2, 3, 4]), Some(2));
    }

    #[test]
    fn payload_serialization_is_byte_equivalent_for_same_inputs() {
        let week_start = ts("2026-05-04T00:00:00Z");
        let patterns = vec![".*incident.*".to_string()];

        let prs = vec![pr(
            1,
            10,
            "2026-05-04T00:00:00Z",
            Some("2026-05-04T10:00:00Z"),
            5,
            5,
            "sha-1",
        )];

        let reviews = vec![review(1, 1, 1, "2026-05-04T01:00:00Z")];
        let commits = vec![commit(
            "sha-1",
            "2026-05-03T23:00:00Z",
            "2026-05-04T00:10:00Z",
            1,
            1,
        )];
        let deployments = vec![deployment(
            1,
            "merge-sha-1",
            "success",
            "2026-05-04T11:00:00Z",
        )];

        let input = WeeklyMetricInput {
            repository_id: 42,
            week_start_utc: week_start,
            metric_version: "v1",
            incident_label_patterns: &patterns,
            pull_requests: &prs,
            reviews: &reviews,
            commits: &commits,
            deployments: &deployments,
            issues: &[],
        };

        let a = WeeklyMetricEngine::compute(input.clone());
        let b = WeeklyMetricEngine::compute(input);

        let payload_a = WeeklyMetricEngine::serialize_payload(&a).expect("serialize a");
        let payload_b = WeeklyMetricEngine::serialize_payload(&b).expect("serialize b");

        assert_eq!(payload_a, payload_b);
    }

    #[test]
    fn cfr_mttr_use_deterministic_tiebreak_and_nearest_forward_recovery() {
        let week_start = ts("2026-05-04T00:00:00Z");
        let patterns = vec![".*incident.*".to_string()];

        let deployments = vec![
            deployment(2, "s", "failure", "2026-05-04T01:00:00Z"),
            deployment(3, "s", "success", "2026-05-04T02:00:00Z"),
            deployment(1, "s", "success", "2026-05-04T03:00:00Z"),
        ];

        let issues = vec![
            issue(10, "open", "2026-05-04T01:00:00Z", None, &["incident"]),
            issue(
                11,
                "closed",
                "2026-05-04T04:00:00Z",
                Some("2026-05-04T04:00:00Z"),
                &["incident"],
            ),
        ];

        let snapshot = WeeklyMetricEngine::compute(WeeklyMetricInput {
            repository_id: 42,
            week_start_utc: week_start,
            metric_version: "v1",
            incident_label_patterns: &patterns,
            pull_requests: &[],
            reviews: &[],
            commits: &[],
            deployments: &deployments,
            issues: &issues,
        });

        assert_eq!(snapshot.metrics.change_failure_rate, Some(1.0 / 3.0));
        assert_eq!(snapshot.metrics.mttr_seconds, Some(3_600));
    }
}
