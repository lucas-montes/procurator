use chrono::{DateTime, Utc};

use crate::{
    adapters::github::dto::{
        GithubCommit, GithubDeployment, GithubIssue, GithubPullRequest, GithubReview,
    },
    domain::signals::{
        NormalizedCommit, NormalizedDeployment, NormalizedIssue, NormalizedPullRequest,
        NormalizedReview,
    },
};

#[derive(Debug, thiserror::Error)]
pub enum TransformError {
    #[error("invalid timestamp '{field}': {value}")]
    InvalidTimestamp { field: &'static str, value: String },
}

fn parse_ts(field: &'static str, value: &str) -> Result<DateTime<Utc>, TransformError> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| TransformError::InvalidTimestamp {
            field,
            value: value.to_string(),
        })
}

fn parse_opt_ts(
    field: &'static str,
    value: &Option<String>,
) -> Result<Option<DateTime<Utc>>, TransformError> {
    match value {
        Some(ts) => parse_ts(field, ts).map(Some),
        None => Ok(None),
    }
}

pub fn normalize_pull_request(
    repository_id: i64,
    dto: &GithubPullRequest,
) -> Result<NormalizedPullRequest, TransformError> {
    Ok(NormalizedPullRequest {
        id: dto.id,
        repository_id,
        number: dto.number,
        title: dto.title.clone(),
        author_id: dto.user.id,
        state: dto.state.clone(),
        created_at: parse_ts("pull_request.created_at", &dto.created_at)?,
        updated_at: parse_ts("pull_request.updated_at", &dto.updated_at)?,
        closed_at: parse_opt_ts("pull_request.closed_at", &dto.closed_at)?,
        merged_at: parse_opt_ts("pull_request.merged_at", &dto.merged_at)?,
        additions: dto.additions,
        deletions: dto.deletions,
        changed_files: dto.changed_files,
        head_sha: dto
            .head
            .as_ref()
            .map_or_else(String::new, |head| head.sha.clone()),
        head_ref: dto
            .head
            .as_ref()
            .map_or_else(String::new, |head| head.ref_field.clone()),
        base_sha: dto
            .base
            .as_ref()
            .map_or_else(String::new, |base| base.sha.clone()),
        base_ref: dto
            .base
            .as_ref()
            .map_or_else(String::new, |base| base.ref_field.clone()),
        merge_commit_sha: dto.merge_commit_sha.clone(),
        draft: dto.draft.unwrap_or(false),
        author_association: dto.author_association.clone(),
    })
}

pub fn normalize_review(
    pull_request_id: i64,
    dto: &GithubReview,
) -> Result<NormalizedReview, TransformError> {
    Ok(NormalizedReview {
        id: dto.id,
        pull_request_id,
        user_id: dto.user.id,
        state: dto.state.clone(),
        submitted_at: parse_ts("review.submitted_at", &dto.submitted_at)?,
        body: dto.body.clone(),
        commit_id: dto.commit_id.clone(),
    })
}

pub fn normalize_commit(
    repository_id: i64,
    dto: &GithubCommit,
) -> Result<NormalizedCommit, TransformError> {
    let additions = dto.stats.as_ref().map_or(0, |stats| stats.additions);
    let deletions = dto.stats.as_ref().map_or(0, |stats| stats.deletions);
    let total = dto
        .stats
        .as_ref()
        .map_or(additions + deletions, |stats| stats.total);

    Ok(NormalizedCommit {
        sha: dto.sha.clone(),
        repository_id,
        author_id: dto.author.as_ref().map_or(0, |user| user.id),
        committer_id: dto.committer.as_ref().map_or(0, |user| user.id),
        message: dto.commit.message.clone(),
        authored_at: parse_ts("commit.author.date", &dto.commit.author.date)?,
        committed_at: parse_ts("commit.committer.date", &dto.commit.committer.date)?,
        additions,
        deletions,
        total,
    })
}

pub fn normalize_deployment(
    repository_id: i64,
    dto: &GithubDeployment,
) -> Result<NormalizedDeployment, TransformError> {
    Ok(NormalizedDeployment {
        id: dto.id,
        repository_id,
        sha: dto.sha.clone(),
        ref_field: dto.ref_field.clone(),
        task: dto.task.clone(),
        payload: dto.payload.clone(),
        environment: dto.environment.clone(),
        state: dto.state.clone(),
        created_at: parse_ts("deployment.created_at", &dto.created_at)?,
        updated_at: parse_ts("deployment.updated_at", &dto.updated_at)?,
        creator_id: dto.creator.as_ref().map(|u| u.id),
        description: dto.description.clone(),
        is_production: dto
            .production_environment
            .unwrap_or_else(|| dto.environment.eq_ignore_ascii_case("production")),
    })
}

pub fn normalize_issue(
    repository_id: i64,
    dto: &GithubIssue,
) -> Result<NormalizedIssue, TransformError> {
    Ok(NormalizedIssue {
        id: dto.id,
        repository_id,
        number: dto.number,
        title: dto.title.clone(),
        author_id: dto.user.id,
        state: dto.state.clone(),
        created_at: parse_ts("issue.created_at", &dto.created_at)?,
        updated_at: parse_ts("issue.updated_at", &dto.updated_at)?,
        closed_at: parse_opt_ts("issue.closed_at", &dto.closed_at)?,
        labels: dto.labels.iter().map(|label| label.name.clone()).collect(),
        is_pull_request: dto.pull_request.is_some(),
        assignee_id: dto.assignee.as_ref().map(|u| u.id),
        milestone_id: dto.milestone.as_ref().map(|m| m.id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::github::dto::{
        GithubCommitAuthor, GithubCommitInfo, GithubCommitStats, GithubLabel, GithubMilestone,
        GithubPullRequestBranch, GithubUser,
    };

    #[test]
    fn normalize_pull_request_maps_branch_and_merge_identity_fields() {
        let dto = GithubPullRequest {
            id: 10,
            number: 55,
            title: "Add dashboard metric".to_string(),
            user: GithubUser {
                id: 7,
                login: "octocat".to_string(),
            },
            state: "closed".to_string(),
            created_at: "2026-05-01T09:00:00Z".to_string(),
            updated_at: "2026-05-02T09:00:00Z".to_string(),
            closed_at: Some("2026-05-02T09:30:00Z".to_string()),
            merged_at: Some("2026-05-02T09:20:00Z".to_string()),
            additions: 40,
            deletions: 10,
            changed_files: 3,
            head: Some(GithubPullRequestBranch {
                sha: "abc123".to_string(),
                ref_field: "feature/metrics".to_string(),
            }),
            base: Some(GithubPullRequestBranch {
                sha: "def456".to_string(),
                ref_field: "main".to_string(),
            }),
            draft: Some(false),
            author_association: Some("CONTRIBUTOR".to_string()),
            merge_commit_sha: Some("fff999".to_string()),
        };

        let normalized = normalize_pull_request(99, &dto).expect("normalizes");
        assert_eq!(normalized.repository_id, 99);
        assert_eq!(normalized.head_sha, "abc123");
        assert_eq!(normalized.base_ref, "main");
        assert_eq!(normalized.merge_commit_sha.as_deref(), Some("fff999"));
        assert!(normalized.is_merged());
    }

    #[test]
    fn normalize_deployment_defaults_production_from_environment_when_flag_missing() {
        let dto = GithubDeployment {
            id: 88,
            sha: "fff999".to_string(),
            ref_field: "refs/heads/main".to_string(),
            task: "deploy".to_string(),
            payload: None,
            environment: "Production".to_string(),
            state: "failure".to_string(),
            created_at: "2026-05-02T10:00:00Z".to_string(),
            updated_at: "2026-05-02T10:02:00Z".to_string(),
            creator: Some(GithubUser {
                id: 20,
                login: "deploy-bot".to_string(),
            }),
            description: Some("Prod rollout".to_string()),
            production_environment: None,
        };

        let normalized = normalize_deployment(99, &dto).expect("normalizes");
        assert!(normalized.is_production());
        assert!(normalized.is_failure());
    }

    #[test]
    fn normalize_issue_maps_incident_metadata_and_linkable_fields() {
        let dto = GithubIssue {
            id: 77,
            number: 13,
            title: "incident: db outage".to_string(),
            user: GithubUser {
                id: 3,
                login: "oncall".to_string(),
            },
            state: "closed".to_string(),
            created_at: "2026-05-02T11:00:00Z".to_string(),
            updated_at: "2026-05-02T12:00:00Z".to_string(),
            closed_at: Some("2026-05-02T12:30:00Z".to_string()),
            labels: vec![GithubLabel {
                id: 1,
                name: "incident".to_string(),
                color: "ff0000".to_string(),
            }],
            pull_request: None,
            assignee: Some(GithubUser {
                id: 99,
                login: "sre".to_string(),
            }),
            milestone: Some(GithubMilestone { id: 404 }),
        };

        let normalized = normalize_issue(99, &dto).expect("normalizes");
        assert_eq!(normalized.labels, vec!["incident".to_string()]);
        assert_eq!(normalized.assignee_id, Some(99));
        assert_eq!(normalized.milestone_id, Some(404));
        assert!(normalized.is_closed());
    }

    #[test]
    fn normalize_review_and_commit_capture_timestamps_and_optional_fields() {
        let review_dto = GithubReview {
            id: 501,
            user: GithubUser {
                id: 14,
                login: "reviewer".to_string(),
            },
            state: "APPROVED".to_string(),
            submitted_at: "2026-05-02T09:10:00Z".to_string(),
            body: Some("LGTM".to_string()),
            commit_id: Some("abc123".to_string()),
        };

        let review = normalize_review(10, &review_dto).expect("review normalizes");
        assert_eq!(review.pull_request_id, 10);
        assert!(review.is_approved());

        let commit_dto = GithubCommit {
            sha: "abc123".to_string(),
            commit: GithubCommitInfo {
                author: GithubCommitAuthor {
                    name: "Dev".to_string(),
                    email: "dev@example.com".to_string(),
                    date: "2026-05-01T08:00:00Z".to_string(),
                },
                committer: GithubCommitAuthor {
                    name: "CI".to_string(),
                    email: "ci@example.com".to_string(),
                    date: "2026-05-01T08:05:00Z".to_string(),
                },
                message: "feat: add DORA signal model".to_string(),
                timestamp: "2026-05-01T08:05:00Z".to_string(),
            },
            author: Some(GithubUser {
                id: 14,
                login: "dev".to_string(),
            }),
            committer: Some(GithubUser {
                id: 15,
                login: "ci-bot".to_string(),
            }),
            stats: Some(GithubCommitStats {
                additions: 30,
                deletions: 5,
                total: 35,
            }),
        };

        let commit = normalize_commit(99, &commit_dto).expect("commit normalizes");
        assert_eq!(commit.author_id, 14);
        assert_eq!(commit.committer_id, 15);
        assert_eq!(commit.total, 35);
        assert_eq!(commit.coding_started_at(), commit.authored_at);
    }
}
