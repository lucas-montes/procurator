use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Normalized submitted review state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Unknown(String),
}

impl ReviewState {
    pub fn from_raw(value: &str) -> Self {
        if value.eq_ignore_ascii_case("approved") {
            Self::Approved
        } else if value.eq_ignore_ascii_case("changes_requested") {
            Self::ChangesRequested
        } else if value.eq_ignore_ascii_case("commented") {
            Self::Commented
        } else if value.eq_ignore_ascii_case("dismissed") {
            Self::Dismissed
        } else {
            Self::Unknown(value.to_string())
        }
    }
}

/// Normalized Review model for cross-forge compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedReview {
    /// Unique identifier for the review
    pub id: i64,
    /// Pull request identifier
    pub pull_request_id: i64,
    /// User who submitted the review
    pub user_id: i64,
    /// Review state as raw forge value
    pub state: String,
    /// When the review was submitted (canonical review-start timestamp)
    pub submitted_at: DateTime<Utc>,
    /// Body/content of the review
    pub body: Option<String>,
    /// Commit ID the review was on
    pub commit_id: Option<String>,
}

impl NormalizedReview {
    /// Normalize review state.
    pub fn state_kind(&self) -> ReviewState {
        ReviewState::from_raw(&self.state)
    }

    /// Canonical review submission timestamp.
    pub fn submitted_timestamp(&self) -> DateTime<Utc> {
        self.submitted_at
    }

    /// Check if review is approved
    pub fn is_approved(&self) -> bool {
        matches!(self.state_kind(), ReviewState::Approved)
    }

    /// Check if review requests changes
    pub fn requests_changes(&self) -> bool {
        matches!(self.state_kind(), ReviewState::ChangesRequested)
    }

    /// Check if review is a comment
    pub fn is_comment(&self) -> bool {
        matches!(self.state_kind(), ReviewState::Commented)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_review_state_case_insensitively() {
        assert_eq!(ReviewState::from_raw("APPROVED"), ReviewState::Approved);
        assert_eq!(
            ReviewState::from_raw("changes_requested"),
            ReviewState::ChangesRequested
        );
    }
}
