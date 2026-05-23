use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Normalized deployment state for failure/recovery metrics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeploymentState {
    Success,
    Failure,
    Inactive,
    Queued,
    InProgress,
    Error,
    Pending,
    Unknown(String),
}

impl DeploymentState {
    pub fn from_raw(value: &str) -> Self {
        if value.eq_ignore_ascii_case("success") {
            Self::Success
        } else if value.eq_ignore_ascii_case("failure") {
            Self::Failure
        } else if value.eq_ignore_ascii_case("inactive") {
            Self::Inactive
        } else if value.eq_ignore_ascii_case("queued") {
            Self::Queued
        } else if value.eq_ignore_ascii_case("in_progress") {
            Self::InProgress
        } else if value.eq_ignore_ascii_case("error") {
            Self::Error
        } else if value.eq_ignore_ascii_case("pending") {
            Self::Pending
        } else {
            Self::Unknown(value.to_string())
        }
    }

    pub fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }

    /// v1 mixed-model semantics: any non-success deployment status is a deployment failure signal.
    pub fn is_failure_signal(self) -> bool {
        !self.is_success()
    }
}

/// Normalized deployment environment classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeploymentEnvironment {
    Production,
    NonProduction,
}

/// Normalized Deployment model for cross-forge compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedDeployment {
    /// Unique identifier for the deployment
    pub id: i64,
    /// Repository identifier
    pub repository_id: i64,
    /// SHA of the commit deployed
    pub sha: String,
    /// Ref/branch/tag deployed
    pub ref_field: String,
    /// Task description
    pub task: String,
    /// Deployment payload (environment-specific data)
    pub payload: Option<serde_json::Value>,
    /// Environment (production, staging, etc.) as raw source value
    pub environment: String,
    /// Deployment state as raw source value
    pub state: String,
    /// When the deployment was created (canonical deployment event timestamp)
    pub created_at: DateTime<Utc>,
    /// Last sync/update timestamp from source
    pub updated_at: DateTime<Utc>,
    /// User who created the deployment
    pub creator_id: Option<i64>,
    /// Description of the deployment
    pub description: Option<String>,
    /// Whether this is a production environment
    pub is_production: bool,
}

impl NormalizedDeployment {
    /// Normalize deployment state.
    pub fn state_kind(&self) -> DeploymentState {
        DeploymentState::from_raw(&self.state)
    }

    /// Normalize deployment environment class.
    pub fn environment_kind(&self) -> DeploymentEnvironment {
        if self.is_production {
            DeploymentEnvironment::Production
        } else {
            DeploymentEnvironment::NonProduction
        }
    }

    /// Canonical deployment timestamp.
    pub fn deployed_at(&self) -> DateTime<Utc> {
        self.created_at.clone()
    }

    /// Check if deployment is successful
    pub fn is_success(&self) -> bool {
        self.state_kind().is_success()
    }

    /// Check if deployment contributes a failure signal in v1 mixed model.
    pub fn is_failure(&self) -> bool {
        self.state_kind().is_failure_signal()
    }

    /// Check if deployment is for production environment
    pub fn is_production(&self) -> bool {
        matches!(self.environment_kind(), DeploymentEnvironment::Production)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_success_deployment_is_failure_signal() {
        assert!(DeploymentState::from_raw("failure").is_failure_signal());
        assert!(DeploymentState::from_raw("inactive").is_failure_signal());
        assert!(!DeploymentState::from_raw("success").is_failure_signal());
    }
}
