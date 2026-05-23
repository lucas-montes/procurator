use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Normalized issue lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueState {
    Open,
    Closed,
    Unknown(String),
}

impl IssueState {
    pub fn from_raw(value: &str) -> Self {
        if value.eq_ignore_ascii_case("open") {
            Self::Open
        } else if value.eq_ignore_ascii_case("closed") {
            Self::Closed
        } else {
            Self::Unknown(value.to_string())
        }
    }
}

/// Classifies incident role in mixed-model failure/recovery calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IncidentRole {
    FailureSignal,
    RecoverySignal,
}

/// Normalized Issue model for cross-forge compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedIssue {
    /// Unique identifier for the issue
    pub id: i64,
    /// Repository identifier
    pub repository_id: i64,
    /// Issue number within the repository
    pub number: i32,
    /// Issue title
    pub title: String,
    /// Author of the issue
    pub author_id: i64,
    /// Current state as raw forge value
    pub state: String,
    /// When the issue was created (canonical opened timestamp)
    pub created_at: DateTime<Utc>,
    /// Last sync/update timestamp from source
    pub updated_at: DateTime<Utc>,
    /// When the issue was closed (if applicable)
    pub closed_at: Option<DateTime<Utc>>,
    /// Labels attached to the issue
    pub labels: Vec<String>,
    /// Whether this issue is a pull request
    pub is_pull_request: bool,
    /// Assignee of the issue
    pub assignee_id: Option<i64>,
    /// Milestone associated with the issue
    pub milestone_id: Option<i64>,
}

impl NormalizedIssue {
    /// Normalize issue state.
    pub fn state_kind(&self) -> IssueState {
        IssueState::from_raw(&self.state)
    }

    /// Canonical issue-opened timestamp.
    pub fn opened_at(&self) -> DateTime<Utc> {
        self.created_at.clone()
    }

    /// Canonical issue-closed timestamp.
    pub fn closed_timestamp(&self) -> Option<DateTime<Utc>> {
        self.closed_at.clone()
    }

    /// Check if issue is open
    pub fn is_open(&self) -> bool {
        matches!(self.state_kind(), IssueState::Open)
    }

    /// Check if issue is closed
    pub fn is_closed(&self) -> bool {
        matches!(self.state_kind(), IssueState::Closed)
    }

    /// Check if issue has a specific label
    pub fn has_label(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
    }

    /// Check if issue is an incident (based on label convention)
    pub fn is_incident(&self, incident_label_patterns: &[String]) -> bool {
        incident_label_patterns.iter().any(|pattern| {
            self.labels.iter().any(|label| {
                // Simple wildcard matching for common patterns
                if pattern == ".*incident.*" {
                    label.to_lowercase().contains("incident")
                } else if pattern == ".*bug.*" {
                    label.to_lowercase().contains("bug")
                } else if pattern == ".*outage.*" {
                    label.to_lowercase().contains("outage")
                } else {
                    label == pattern
                }
            })
        })
    }

    /// Mixed-model failure/recovery role for incident-labelled issues.
    pub fn incident_role(&self, incident_label_patterns: &[String]) -> Option<IncidentRole> {
        if !self.is_incident(incident_label_patterns) {
            return None;
        }

        if self.is_closed() {
            Some(IncidentRole::RecoverySignal)
        } else {
            Some(IncidentRole::FailureSignal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_issue(state: &str, labels: Vec<&str>) -> NormalizedIssue {
        NormalizedIssue {
            id: 1,
            repository_id: 2,
            number: 3,
            title: "incident".to_string(),
            author_id: 4,
            state: state.to_string(),
            created_at: DateTime::parse_from_rfc3339("2026-05-10T10:00:00Z")
                .expect("valid")
                .with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339("2026-05-10T11:00:00Z")
                .expect("valid")
                .with_timezone(&Utc),
            closed_at: if state.eq_ignore_ascii_case("closed") {
                Some(
                    DateTime::parse_from_rfc3339("2026-05-10T12:00:00Z")
                        .expect("valid")
                        .with_timezone(&Utc),
                )
            } else {
                None
            },
            labels: labels.into_iter().map(ToString::to_string).collect(),
            is_pull_request: false,
            assignee_id: None,
            milestone_id: None,
        }
    }

    #[test]
    fn incident_role_maps_open_to_failure_and_closed_to_recovery() {
        let patterns = vec![".*incident.*".to_string()];
        let open_incident = sample_issue("open", vec!["incident"]);
        let closed_incident = sample_issue("closed", vec!["incident"]);

        assert_eq!(
            open_incident.incident_role(&patterns),
            Some(IncidentRole::FailureSignal)
        );
        assert_eq!(
            closed_incident.incident_role(&patterns),
            Some(IncidentRole::RecoverySignal)
        );
    }
}
