use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::signals::{IncidentRole, NormalizedDeployment, NormalizedIssue};

/// Origin of a mixed-model failure/recovery signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureSignalSource {
    Deployment,
    Incident,
}

/// Canonical mixed-model signal event used by CFR/MTTR computations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureSignal {
    pub repository_id: i64,
    pub source: FailureSignalSource,
    pub occurred_at: DateTime<Utc>,
    pub is_recovery: bool,
}

impl FailureSignal {
    pub fn is_failure(&self) -> bool {
        !self.is_recovery
    }
}

impl NormalizedDeployment {
    /// Convert a production deployment into a failure/recovery signal when applicable.
    pub fn to_failure_signal(&self) -> Option<FailureSignal> {
        if !self.is_production() {
            return None;
        }

        Some(FailureSignal {
            repository_id: self.repository_id,
            source: FailureSignalSource::Deployment,
            occurred_at: self.deployed_at(),
            is_recovery: self.is_success(),
        })
    }
}

impl NormalizedIssue {
    /// Convert incident-labelled issues into failure/recovery signals for mixed model.
    pub fn to_failure_signal(&self, incident_label_patterns: &[String]) -> Option<FailureSignal> {
        let incident_role = self.incident_role(incident_label_patterns)?;
        let occurred_at = match incident_role {
            IncidentRole::FailureSignal => self.opened_at(),
            IncidentRole::RecoverySignal => self.closed_timestamp()?,
        };

        Some(FailureSignal {
            repository_id: self.repository_id,
            source: FailureSignalSource::Incident,
            occurred_at,
            is_recovery: matches!(incident_role, IncidentRole::RecoverySignal),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::signals::NormalizedIssue;

    fn parse_dt(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("valid ts")
            .with_timezone(&Utc)
    }

    #[test]
    fn production_deployment_maps_to_recovery_on_success() {
        let deployment = NormalizedDeployment {
            id: 1,
            repository_id: 100,
            sha: "abc123".to_string(),
            ref_field: "refs/heads/main".to_string(),
            task: "deploy".to_string(),
            payload: None,
            environment: "production".to_string(),
            state: "success".to_string(),
            created_at: parse_dt("2026-05-10T10:00:00Z"),
            updated_at: parse_dt("2026-05-10T10:05:00Z"),
            creator_id: Some(42),
            description: None,
            is_production: true,
        };

        let signal = deployment.to_failure_signal().expect("signal generated");
        assert_eq!(signal.source, FailureSignalSource::Deployment);
        assert!(signal.is_recovery);
        assert!(!signal.is_failure());
    }

    #[test]
    fn incident_issue_maps_to_failure_and_recovery() {
        let patterns = vec![".*incident.*".to_string()];

        let open_incident = NormalizedIssue {
            id: 1,
            repository_id: 100,
            number: 5,
            title: "outage".to_string(),
            author_id: 7,
            state: "open".to_string(),
            created_at: parse_dt("2026-05-10T09:00:00Z"),
            updated_at: parse_dt("2026-05-10T09:01:00Z"),
            closed_at: None,
            labels: vec!["incident".to_string()],
            is_pull_request: false,
            assignee_id: None,
            milestone_id: None,
        };

        let closed_incident = NormalizedIssue {
            state: "closed".to_string(),
            closed_at: Some(parse_dt("2026-05-10T12:00:00Z")),
            ..open_incident.clone()
        };

        let failure = open_incident
            .to_failure_signal(&patterns)
            .expect("failure signal");
        let recovery = closed_incident
            .to_failure_signal(&patterns)
            .expect("recovery signal");

        assert!(failure.is_failure());
        assert!(!failure.is_recovery);
        assert!(recovery.is_recovery);
        assert_eq!(recovery.occurred_at, parse_dt("2026-05-10T12:00:00Z"));
    }
}
