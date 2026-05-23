use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Normalized pull request lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
    Unknown(String),
}

impl PullRequestState {
    pub fn from_raw(value: &str, merged_at: Option<&DateTime<Utc>>) -> Self {
        if merged_at.is_some() {
            return Self::Merged;
        }

        if value.eq_ignore_ascii_case("open") {
            Self::Open
        } else if value.eq_ignore_ascii_case("closed") {
            Self::Closed
        } else {
            Self::Unknown(value.to_string())
        }
    }
}

/// Normalized Pull Request model for cross-forge compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedPullRequest {
    /// Unique identifier for the PR
    pub id: i64,
    /// Repository identifier
    pub repository_id: i64,
    /// PR number within the repository
    pub number: i32,
    /// PR title
    pub title: String,
    /// Author of the PR
    pub author_id: i64,
    /// Current state as raw forge value
    pub state: String,
    /// When the PR was created (canonical "opened" timestamp)
    pub created_at: DateTime<Utc>,
    /// Last sync/update timestamp from source
    pub updated_at: DateTime<Utc>,
    /// When the PR was closed (if applicable)
    pub closed_at: Option<DateTime<Utc>>,
    /// When the PR was merged (if applicable)
    pub merged_at: Option<DateTime<Utc>>,
    /// Number of additions in the PR
    pub additions: u32,
    /// Number of deletions in the PR
    pub deletions: u32,
    /// Number of changed files
    pub changed_files: u32,
    /// Head commit SHA (tip of the PR branch)
    pub head_sha: String,
    /// Head branch reference
    pub head_ref: String,
    /// Base commit SHA (target branch)
    pub base_sha: String,
    /// Base branch reference
    pub base_ref: String,
    /// Merge commit SHA when available
    pub merge_commit_sha: Option<String>,
    /// Whether the PR is a draft
    pub draft: bool,
    /// Author association (OWNER, CONTRIBUTOR, etc.)
    pub author_association: Option<String>,
}

impl NormalizedPullRequest {
    /// Calculate PR size (additions + deletions)
    pub fn size(&self) -> u32 {
        self.additions + self.deletions
    }

    /// Normalized lifecycle state derived from raw state and merge timestamp.
    pub fn state_kind(&self) -> PullRequestState {
        PullRequestState::from_raw(&self.state, self.merged_at.as_ref())
    }

    /// Canonical timestamp for PR-opened metrics.
    pub fn opened_at(&self) -> DateTime<Utc> {
        self.created_at.clone()
    }

    /// Canonical timestamp for PR merged metrics.
    pub fn merged_timestamp(&self) -> Option<DateTime<Utc>> {
        self.merged_at.clone()
    }

    /// Canonical terminal timestamp: merge time when merged, otherwise close time.
    pub fn terminal_at(&self) -> Option<DateTime<Utc>> {
        self.merged_at.clone().or(self.closed_at.clone())
    }

    /// Check if PR is merged
    pub fn is_merged(&self) -> bool {
        matches!(self.state_kind(), PullRequestState::Merged)
    }

    /// Check if PR is open
    pub fn is_open(&self) -> bool {
        matches!(self.state_kind(), PullRequestState::Open)
    }

    /// Check if PR is closed (but not merged)
    pub fn is_closed(&self) -> bool {
        matches!(self.state_kind(), PullRequestState::Closed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_merged_state_from_timestamp_even_if_raw_state_closed() {
        let merged_at = DateTime::parse_from_rfc3339("2026-05-10T10:00:00Z")
            .expect("valid ts")
            .with_timezone(&Utc);

        let state = PullRequestState::from_raw("closed", Some(&merged_at));
        assert_eq!(state, PullRequestState::Merged);
    }
}
