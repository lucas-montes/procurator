use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use tracing::{info, instrument, warn};

use crate::adapters::shared::database::{Database, DatabaseError};
use crate::application::ports::{
    ForgeError, ForgeRepositoryTarget, ForgeSignalPort, NormalizedSignalBatch,
};
use crate::domain::metrics::{WEEKLY_WINDOW_DAYS, WeeklyMetricEngine, WeeklyMetricInput};

/// Errors surfaced by the refresh orchestration pipeline.
#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error("forge port error: {0}")]
    Forge(#[from] ForgeError),

    #[error("database error: {0}")]
    Database(#[from] DatabaseError),

    #[error("no data returned for repository {repository_id}")]
    NoData { repository_id: i64 },
}

/// Summary of a completed refresh cycle.
#[derive(Debug, Clone)]
pub struct RefreshResult {
    pub repository_id: i64,
    pub weeks_covered: usize,
    pub week_summaries: Vec<RefreshWeekSummary>,
    pub signal_counts: RefreshSignalCounts,
}

/// Per-week summary within a refresh result.
#[derive(Debug, Clone)]
pub struct RefreshWeekSummary {
    pub week_start_utc: String,
    pub signal_count: usize,
    pub snapshot_persisted: bool,
}

/// Counts of each signal type fetched during refresh.
#[derive(Debug, Clone, Default)]
pub struct RefreshSignalCounts {
    pub pull_requests: usize,
    pub reviews: usize,
    pub commits: usize,
    pub deployments: usize,
    pub issues: usize,
}

/// Orchestrates the on-demand refresh pipeline:
/// fetch (via forge port) → persist normalized signals → compute per-week metrics → persist snapshots.
pub struct RefreshOrchestrator {
    port: Box<dyn ForgeSignalPort>,
    db: Database,
}

impl RefreshOrchestrator {
    /// Create a new orchestrator with a forge-agnostic signal port and database.
    pub fn new(port: Box<dyn ForgeSignalPort>, db: Database) -> Self {
        Self { port, db }
    }

    /// Execute a full refresh cycle for the given repository target.
    ///
    /// Returns a [`RefreshResult`] summarising what was fetched, computed, and persisted.
    /// Errors are surfaced via [`RefreshError`] with structured `tracing` logging at each stage.
    #[instrument(skip(self), fields(repository_id = %target.repository_id))]
    pub async fn trigger_refresh(
        &self,
        target: &ForgeRepositoryTarget,
        incident_label_patterns: &[String],
        metric_version: &str,
    ) -> Result<RefreshResult, RefreshError> {
        info!(
            repository_id = target.repository_id,
            owner = %target.owner,
            name = %target.name,
            "Starting refresh"
        );

        // ── Step 1: Fetch all signals via port ──────────────────────────
        let batch = self.port.fetch_all(target).await?;

        let signal_counts = RefreshSignalCounts {
            pull_requests: batch.pull_requests.len(),
            reviews: batch.reviews.len(),
            commits: batch.commits.len(),
            deployments: batch.deployments.len(),
            issues: batch.issues.len(),
        };

        info!(
            pr_count = signal_counts.pull_requests,
            review_count = signal_counts.reviews,
            commit_count = signal_counts.commits,
            deployment_count = signal_counts.deployments,
            issue_count = signal_counts.issues,
            "Fetched normalized signals"
        );

        if signal_counts.pull_requests == 0
            && signal_counts.reviews == 0
            && signal_counts.commits == 0
            && signal_counts.deployments == 0
            && signal_counts.issues == 0
        {
            warn!(
                repository_id = target.repository_id,
                "No signals returned; aborting refresh"
            );
            return Err(RefreshError::NoData {
                repository_id: target.repository_id,
            });
        }

        // ── Step 2: Persist normalized signals (upsert — idempotent) ────
        persist_signals(&self.db, &batch).await?;

        // ── Step 3: Detect week windows from signal timestamps ──────────
        let week_starts = detect_week_windows(&batch);

        info!(
            weeks = week_starts.len(),
            "Detected week windows for computation"
        );

        // ── Step 4: Compute per-week metrics and persist snapshots ──────
        let mut week_summaries = Vec::with_capacity(week_starts.len());

        for week_start_utc in &week_starts {
            let snapshot = WeeklyMetricEngine::compute(WeeklyMetricInput {
                repository_id: target.repository_id,
                week_start_utc: *week_start_utc,
                metric_version,
                incident_label_patterns,
                pull_requests: &batch.pull_requests,
                reviews: &batch.reviews,
                commits: &batch.commits,
                deployments: &batch.deployments,
                issues: &batch.issues,
            });

            let serialized = WeeklyMetricEngine::serialize_payload(&snapshot)
                .map_err(|e| DatabaseError::InvalidData(e.to_string()))?;

            let computed_at = Utc::now().to_rfc3339();

            self.db
                .upsert_weekly_metric_snapshot(
                    target.repository_id,
                    &snapshot.week_start_utc,
                    metric_version,
                    &serialized,
                    &computed_at,
                )
                .await?;

            week_summaries.push(RefreshWeekSummary {
                week_start_utc: snapshot.week_start_utc.clone(),
                signal_count: count_signals_in_window(&batch, *week_start_utc),
                snapshot_persisted: true,
            });

            info!(
                week = %snapshot.week_start_utc,
                signal_count = week_summaries.last().map(|s| s.signal_count).unwrap_or(0),
                "Computed and persisted weekly metric snapshot"
            );
        }

        info!(
            repository_id = target.repository_id,
            weeks = week_summaries.len(),
            "Refresh completed successfully"
        );

        Ok(RefreshResult {
            repository_id: target.repository_id,
            weeks_covered: week_starts.len(),
            week_summaries,
            signal_counts,
        })
    }
}

// ── Private helpers ──────────────────────────────────────────────────────────

async fn persist_signals(db: &Database, batch: &NormalizedSignalBatch) -> Result<(), RefreshError> {
    if !batch.pull_requests.is_empty() {
        db.upsert_normalized_pull_requests(&batch.pull_requests)
            .await?;
        info!(count = batch.pull_requests.len(), "Persisted pull requests");
    }
    if !batch.reviews.is_empty() {
        db.upsert_normalized_reviews(&batch.reviews).await?;
        info!(count = batch.reviews.len(), "Persisted reviews");
    }
    if !batch.commits.is_empty() {
        db.upsert_normalized_commits(&batch.commits).await?;
        info!(count = batch.commits.len(), "Persisted commits");
    }
    if !batch.deployments.is_empty() {
        db.upsert_normalized_deployments(&batch.deployments).await?;
        info!(count = batch.deployments.len(), "Persisted deployments");
    }
    if !batch.issues.is_empty() {
        db.upsert_normalized_issues(&batch.issues).await?;
        info!(count = batch.issues.len(), "Persisted issues");
    }
    Ok(())
}

/// Floor a timestamp to the start of its UTC week (Monday 00:00:00).
fn floor_to_week_start(timestamp: DateTime<Utc>) -> DateTime<Utc> {
    let weekday_num = timestamp.weekday().num_days_from_monday() as i64;
    let days_to_sub = Duration::days(weekday_num);
    let secs_since_midnight = Duration::seconds(timestamp.timestamp() % 86400);
    (timestamp - days_to_sub - secs_since_midnight)
        .with_nanosecond(0)
        .expect("valid datetime after truncation")
}

/// Detect all unique week windows covered by the signals in the batch.
fn detect_week_windows(batch: &NormalizedSignalBatch) -> Vec<DateTime<Utc>> {
    let mut timestamps: Vec<DateTime<Utc>> = Vec::new();

    for pr in &batch.pull_requests {
        timestamps.push(pr.opened_at());
        if let Some(merged) = pr.merged_timestamp() {
            timestamps.push(merged);
        }
    }
    for review in &batch.reviews {
        timestamps.push(review.submitted_timestamp());
    }
    for commit in &batch.commits {
        timestamps.push(commit.committed_timestamp());
    }
    for deployment in &batch.deployments {
        timestamps.push(deployment.deployed_at());
    }
    for issue in &batch.issues {
        timestamps.push(issue.opened_at());
    }

    if timestamps.is_empty() {
        return Vec::new();
    }

    let min_ts = timestamps.iter().min().expect("non-empty");
    let max_ts = timestamps.iter().max().expect("non-empty");

    let mut week_start = floor_to_week_start(*min_ts);
    let end = floor_to_week_start(*max_ts);
    let mut weeks = Vec::new();

    loop {
        weeks.push(week_start);
        if week_start >= end {
            break;
        }
        week_start = week_start + Duration::days(WEEKLY_WINDOW_DAYS);
    }

    weeks
}

/// Count signals whose primary timestamp falls within a given week window
/// [`week_start`, `week_start` + 7 days).
fn count_signals_in_window(batch: &NormalizedSignalBatch, week_start: DateTime<Utc>) -> usize {
    let week_end = week_start + Duration::days(WEEKLY_WINDOW_DAYS);

    let mut count = 0usize;

    count += batch
        .pull_requests
        .iter()
        .filter(|pr| {
            let ts = pr.opened_at();
            ts >= week_start && ts < week_end
        })
        .count();

    count += batch
        .reviews
        .iter()
        .filter(|review| {
            let ts = review.submitted_timestamp();
            ts >= week_start && ts < week_end
        })
        .count();

    count += batch
        .commits
        .iter()
        .filter(|commit| {
            let ts = commit.committed_timestamp();
            ts >= week_start && ts < week_end
        })
        .count();

    count += batch
        .deployments
        .iter()
        .filter(|deployment| {
            let ts = deployment.deployed_at();
            ts >= week_start && ts < week_end
        })
        .count();

    count += batch
        .issues
        .iter()
        .filter(|issue| {
            let ts = issue.opened_at();
            ts >= week_start && ts < week_end
        })
        .count();

    count
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::shared::database::Database;
    use crate::application::ports::{NormalizedSignalBatch, PortFuture};
    use crate::domain::signals::{
        NormalizedCommit, NormalizedDeployment, NormalizedIssue, NormalizedPullRequest,
        NormalizedReview,
    };

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn ts(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("valid rfc3339 timestamp")
            .with_timezone(&Utc)
    }

    fn make_target(repo_id: i64) -> ForgeRepositoryTarget {
        ForgeRepositoryTarget {
            repository_id: repo_id,
            owner: "test-owner".to_string(),
            name: "test-repo".to_string(),
        }
    }

    fn make_pr(id: i64, repo_id: i64, opened: &str) -> NormalizedPullRequest {
        NormalizedPullRequest {
            id,
            repository_id: repo_id,
            number: id as i32,
            title: format!("PR {id}"),
            author_id: 900 + id,
            state: "open".to_string(),
            created_at: ts(opened),
            updated_at: ts(opened),
            closed_at: None,
            merged_at: None,
            additions: 10,
            deletions: 2,
            changed_files: 1,
            head_sha: format!("head-{id}"),
            head_ref: "feature/x".to_string(),
            base_sha: "base".to_string(),
            base_ref: "main".to_string(),
            merge_commit_sha: None,
            draft: false,
            author_association: None,
        }
    }

    fn make_review(id: i64, pr_id: i64, submitted: &str) -> NormalizedReview {
        NormalizedReview {
            id,
            pull_request_id: pr_id,
            user_id: 100,
            state: "approved".to_string(),
            submitted_at: ts(submitted),
            body: None,
            commit_id: None,
        }
    }

    fn make_commit(sha: &str, repo_id: i64, committed: &str) -> NormalizedCommit {
        NormalizedCommit {
            sha: sha.to_string(),
            repository_id: repo_id,
            author_id: 1,
            committer_id: 1,
            message: "commit".to_string(),
            authored_at: ts(committed),
            committed_at: ts(committed),
            additions: 5,
            deletions: 2,
            total: 7,
        }
    }

    fn make_deployment(id: i64, repo_id: i64, sha: &str, at: &str) -> NormalizedDeployment {
        NormalizedDeployment {
            id,
            repository_id: repo_id,
            sha: sha.to_string(),
            ref_field: "refs/heads/main".to_string(),
            task: "deploy".to_string(),
            payload: None,
            environment: "production".to_string(),
            state: "success".to_string(),
            created_at: ts(at),
            updated_at: ts(at),
            creator_id: Some(1),
            description: None,
            is_production: true,
        }
    }

    fn make_issue(id: i64, repo_id: i64, opened: &str, labels: &[&str]) -> NormalizedIssue {
        NormalizedIssue {
            id,
            repository_id: repo_id,
            number: id as i32,
            title: format!("issue {id}"),
            author_id: 1,
            state: "open".to_string(),
            created_at: ts(opened),
            updated_at: ts(opened),
            closed_at: None,
            labels: labels.iter().map(|l| l.to_string()).collect(),
            is_pull_request: false,
            assignee_id: None,
            milestone_id: None,
        }
    }

    /// Pre-create a `github_pull_requests` record so that review upserts
    /// (which resolve repository_id via `resolve_repository_id_for_review`)
    /// can find the PR.  This mirrors the pattern in the existing database tests.
    async fn ensure_github_pr_exists(db: &Database, pr_id: i64, repo_id: i64, opened: &str) {
        db.create_github_pull_request(
            pr_id, // stored as github_id
            repo_id,
            pr_id as i32, // number
            &format!("PR {pr_id}"),
            Some(1), // user_id of the test user created in each test
            "open",
            opened,
            opened,
            None,
            None,
            10u32,
            2u32,
            1u32,
        )
        .await
        .expect("create github_pull_requests record");
    }

    // ── Mock Port ────────────────────────────────────────────────────────────

    struct MockSignalPort {
        batch: NormalizedSignalBatch,
        fail: bool,
    }

    impl MockSignalPort {
        fn happy(batch: NormalizedSignalBatch) -> Self {
            Self { batch, fail: false }
        }

        fn failing() -> Self {
            Self {
                batch: NormalizedSignalBatch::default(),
                fail: true,
            }
        }
    }

    impl ForgeSignalPort for MockSignalPort {
        fn fetch_pull_requests<'a>(
            &'a self,
            _target: &'a ForgeRepositoryTarget,
        ) -> PortFuture<'a, Result<Vec<NormalizedPullRequest>, ForgeError>> {
            Box::pin(async move {
                if self.fail {
                    Err(ForgeError::Upstream("mock failure".to_string()))
                } else {
                    Ok(self.batch.pull_requests.clone())
                }
            })
        }

        fn fetch_reviews<'a>(
            &'a self,
            _target: &'a ForgeRepositoryTarget,
        ) -> PortFuture<'a, Result<Vec<NormalizedReview>, ForgeError>> {
            Box::pin(async move {
                if self.fail {
                    Err(ForgeError::Upstream("mock failure".to_string()))
                } else {
                    Ok(self.batch.reviews.clone())
                }
            })
        }

        fn fetch_commits<'a>(
            &'a self,
            _target: &'a ForgeRepositoryTarget,
        ) -> PortFuture<'a, Result<Vec<NormalizedCommit>, ForgeError>> {
            Box::pin(async move {
                if self.fail {
                    Err(ForgeError::Upstream("mock failure".to_string()))
                } else {
                    Ok(self.batch.commits.clone())
                }
            })
        }

        fn fetch_deployments<'a>(
            &'a self,
            _target: &'a ForgeRepositoryTarget,
        ) -> PortFuture<'a, Result<Vec<NormalizedDeployment>, ForgeError>> {
            Box::pin(async move {
                if self.fail {
                    Err(ForgeError::Upstream("mock failure".to_string()))
                } else {
                    Ok(self.batch.deployments.clone())
                }
            })
        }

        fn fetch_issues<'a>(
            &'a self,
            _target: &'a ForgeRepositoryTarget,
        ) -> PortFuture<'a, Result<Vec<NormalizedIssue>, ForgeError>> {
            Box::pin(async move {
                if self.fail {
                    Err(ForgeError::Upstream("mock failure".to_string()))
                } else {
                    Ok(self.batch.issues.clone())
                }
            })
        }

        fn fetch_all<'a>(
            &'a self,
            _target: &'a ForgeRepositoryTarget,
        ) -> PortFuture<'a, Result<NormalizedSignalBatch, ForgeError>> {
            Box::pin(async move {
                if self.fail {
                    Err(ForgeError::Upstream("mock failure".to_string()))
                } else {
                    Ok(self.batch.clone())
                }
            })
        }
    }

    // ── Tests ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn trigger_refresh_persists_signals_and_snapshots() {
        let db = Database::new("sqlite::memory:").await.expect("db");

        let user_id = db.create_user("test", None).await.expect("user");
        let project_id = db
            .create_project("test-proj", user_id, None)
            .await
            .expect("project");
        let repo_id = db
            .create_repository(project_id, "test-repo", "https://example.invalid/repo.git")
            .await
            .expect("repo");

        let target = make_target(repo_id);
        let patterns = vec![".*incident.*".to_string()];

        // Pre-create github_pull_requests records so that review upserts can
        // resolve repository_id.
        ensure_github_pr_exists(&db, 1, repo_id, "2026-05-04T00:00:00Z").await;
        ensure_github_pr_exists(&db, 2, repo_id, "2026-05-11T00:00:00Z").await;

        let batch = NormalizedSignalBatch {
            pull_requests: vec![
                make_pr(1, repo_id, "2026-05-04T00:00:00Z"),
                make_pr(2, repo_id, "2026-05-11T00:00:00Z"),
            ],
            reviews: vec![make_review(1, 1, "2026-05-04T02:00:00Z")],
            commits: vec![make_commit("sha-1", repo_id, "2026-05-03T22:00:00Z")],
            deployments: vec![make_deployment(1, repo_id, "sha-1", "2026-05-05T00:00:00Z")],
            issues: vec![make_issue(
                1,
                repo_id,
                "2026-05-05T00:00:00Z",
                &["incident"],
            )],
        };

        let port = MockSignalPort::happy(batch);
        let orchestrator = RefreshOrchestrator::new(Box::new(port), db.clone());

        let result = orchestrator
            .trigger_refresh(&target, &patterns, "v1")
            .await
            .expect("refresh should succeed");

        assert_eq!(result.repository_id, repo_id);
        assert_eq!(result.signal_counts.pull_requests, 2);
        assert_eq!(result.signal_counts.reviews, 1);
        assert_eq!(result.signal_counts.commits, 1);
        assert_eq!(result.signal_counts.deployments, 1);
        assert_eq!(result.signal_counts.issues, 1);
        assert!(result.weeks_covered >= 1, "should cover at least one week");

        // Verify signals persisted in normalized_signals table
        let signal_rows = db
            .list_normalized_signals_by_repository(repo_id, 100)
            .await
            .expect("list signals");
        assert_eq!(signal_rows.len(), 6, "all signal types persisted");

        // Verify snapshots persisted
        let snapshot_rows = db
            .list_weekly_metric_snapshots_by_repository(repo_id, 100)
            .await
            .expect("list snapshots");
        assert!(!snapshot_rows.is_empty(), "at least one snapshot persisted");
        for row in &snapshot_rows {
            assert_eq!(row.window_days, 7);
            assert_eq!(row.metric_version, "v1");
        }
    }

    #[tokio::test]
    async fn trigger_refresh_is_idempotent() {
        let db = Database::new("sqlite::memory:").await.expect("db");

        let user_id = db.create_user("test", None).await.expect("user");
        let project_id = db
            .create_project("test-proj", user_id, None)
            .await
            .expect("project");
        let repo_id = db
            .create_repository(project_id, "test-repo", "https://example.invalid/repo.git")
            .await
            .expect("repo");

        let target = make_target(repo_id);
        let patterns = vec![".*incident.*".to_string()];

        ensure_github_pr_exists(&db, 5, repo_id, "2026-05-04T00:00:00Z").await;

        let batch = NormalizedSignalBatch {
            pull_requests: vec![make_pr(5, repo_id, "2026-05-04T00:00:00Z")],
            reviews: vec![],
            commits: vec![],
            deployments: vec![],
            issues: vec![],
        };

        // First refresh
        {
            let port = MockSignalPort::happy(batch.clone());
            let orchestrator = RefreshOrchestrator::new(Box::new(port), db.clone());
            orchestrator
                .trigger_refresh(&target, &patterns, "v1")
                .await
                .expect("first refresh");
        }

        let signals_first = db
            .list_normalized_signals_by_repository(repo_id, 100)
            .await
            .expect("list signals");
        let snapshots_first = db
            .list_weekly_metric_snapshots_by_repository(repo_id, 100)
            .await
            .expect("list snapshots");

        // Second refresh (same data)
        {
            let port = MockSignalPort::happy(batch);
            let orchestrator = RefreshOrchestrator::new(Box::new(port), db.clone());
            orchestrator
                .trigger_refresh(&target, &patterns, "v1")
                .await
                .expect("second refresh");
        }

        let signals_second = db
            .list_normalized_signals_by_repository(repo_id, 100)
            .await
            .expect("list signals after second refresh");
        let snapshots_second = db
            .list_weekly_metric_snapshots_by_repository(repo_id, 100)
            .await
            .expect("list snapshots after second refresh");

        // Counts must be identical (idempotent upsert)
        assert_eq!(
            signals_first.len(),
            signals_second.len(),
            "signal count unchanged after re-trigger"
        );
        assert_eq!(
            snapshots_first.len(),
            snapshots_second.len(),
            "snapshot count unchanged after re-trigger"
        );
    }

    #[tokio::test]
    async fn trigger_refresh_propagates_port_failure() {
        let db = Database::new("sqlite::memory:").await.expect("db");
        let target = make_target(42);
        let patterns = vec![];
        let port = MockSignalPort::failing();
        let orchestrator = RefreshOrchestrator::new(Box::new(port), db);

        let result = orchestrator.trigger_refresh(&target, &patterns, "v1").await;

        match result {
            Err(RefreshError::Forge(ForgeError::Upstream(msg))) => {
                assert_eq!(msg, "mock failure");
            }
            other => panic!("expected RefreshError::Forge(Upstream), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn trigger_refresh_errors_on_empty_data() {
        let db = Database::new("sqlite::memory:").await.expect("db");

        let user_id = db.create_user("test", None).await.expect("user");
        let project_id = db
            .create_project("test-proj", user_id, None)
            .await
            .expect("project");
        let repo_id = db
            .create_repository(project_id, "test-repo", "https://example.invalid/repo.git")
            .await
            .expect("repo");

        let target = make_target(repo_id);
        let patterns = vec![];

        let port = MockSignalPort::happy(NormalizedSignalBatch::default());
        let orchestrator = RefreshOrchestrator::new(Box::new(port), db);

        let result = orchestrator.trigger_refresh(&target, &patterns, "v1").await;

        match result {
            Err(RefreshError::NoData { repository_id }) => {
                assert_eq!(repository_id, repo_id);
            }
            other => panic!("expected RefreshError::NoData, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn week_detection_spans_multiple_windows() {
        let db = Database::new("sqlite::memory:").await.expect("db");

        let user_id = db.create_user("test", None).await.expect("user");
        let project_id = db
            .create_project("test-proj", user_id, None)
            .await
            .expect("project");
        let repo_id = db
            .create_repository(project_id, "test-repo", "https://example.invalid/repo.git")
            .await
            .expect("repo");

        let target = make_target(repo_id);
        let patterns = vec![];

        // Two PRs three weeks apart
        ensure_github_pr_exists(&db, 10, repo_id, "2026-05-04T00:00:00Z").await;
        ensure_github_pr_exists(&db, 11, repo_id, "2026-05-25T00:00:00Z").await;

        let batch = NormalizedSignalBatch {
            pull_requests: vec![
                make_pr(10, repo_id, "2026-05-04T00:00:00Z"),
                make_pr(11, repo_id, "2026-05-25T00:00:00Z"),
            ],
            reviews: vec![],
            commits: vec![],
            deployments: vec![],
            issues: vec![],
        };

        let port = MockSignalPort::happy(batch);
        let orchestrator = RefreshOrchestrator::new(Box::new(port), db.clone());

        let result = orchestrator
            .trigger_refresh(&target, &patterns, "v1")
            .await
            .expect("multi-week refresh");

        // Should cover weeks: May 4, May 11, May 18, May 25 = 4 weeks
        assert_eq!(result.weeks_covered, 4, "three-week span yields 4 windows");
        assert_eq!(result.week_summaries.len(), 4);

        // Verify persisted snapshot weeks
        let snapshot_rows = db
            .list_weekly_metric_snapshots_by_repository(repo_id, 100)
            .await
            .expect("list snapshots");
        assert_eq!(snapshot_rows.len(), 4);
    }

    #[tokio::test]
    async fn floor_to_week_start_rounds_correctly() {
        // Monday noon → Monday midnight (same day)
        let mon_noon = ts("2026-05-04T12:00:00Z");
        assert_eq!(floor_to_week_start(mon_noon), ts("2026-05-04T00:00:00Z"));

        // Wednesday → Monday
        let wed = ts("2026-05-06T14:30:00Z");
        assert_eq!(floor_to_week_start(wed), ts("2026-05-04T00:00:00Z"));

        // Sunday → Monday of the same ISO week
        let sun = ts("2026-05-10T23:59:59Z");
        assert_eq!(floor_to_week_start(sun), ts("2026-05-04T00:00:00Z"));

        // Monday midnight → itself
        let mon_mid = ts("2026-05-04T00:00:00Z");
        assert_eq!(floor_to_week_start(mon_mid), ts("2026-05-04T00:00:00Z"));
    }
}
