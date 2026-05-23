use serde::{Deserialize, Serialize};

use crate::domain::signals::{NormalizedDeployment, NormalizedIssue, NormalizedPullRequest};

/// Stable identity for a pull request across normalized signals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestIdentity {
    pub repository_id: i64,
    pub number: i32,
}

/// Link key candidates that can associate signals across PR/deploy/incident boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalLinkKey {
    PullRequest(PullRequestIdentity),
    CommitSha(String),
    Repository(i64),
}

impl NormalizedPullRequest {
    /// Stable identity for direct PR-scoped linking.
    pub fn identity(&self) -> PullRequestIdentity {
        PullRequestIdentity {
            repository_id: self.repository_id,
            number: self.number,
        }
    }

    /// Link candidates usable by downstream correlation logic.
    pub fn link_keys(&self) -> Vec<SignalLinkKey> {
        let mut keys = vec![SignalLinkKey::PullRequest(self.identity())];

        if !self.head_sha.is_empty() {
            keys.push(SignalLinkKey::CommitSha(self.head_sha.clone()));
        }

        if let Some(merge_sha) = &self.merge_commit_sha {
            if !merge_sha.is_empty() {
                keys.push(SignalLinkKey::CommitSha(merge_sha.clone()));
            }
        }

        keys.push(SignalLinkKey::Repository(self.repository_id));
        keys
    }
}

impl NormalizedDeployment {
    /// Link candidates for deployment correlation to PR signals.
    pub fn link_keys(&self) -> Vec<SignalLinkKey> {
        vec![
            SignalLinkKey::CommitSha(self.sha.clone()),
            SignalLinkKey::Repository(self.repository_id),
        ]
    }
}

impl NormalizedIssue {
    /// Incident issues link at repository scope in v1.
    pub fn link_keys(&self) -> Vec<SignalLinkKey> {
        vec![SignalLinkKey::Repository(self.repository_id)]
    }
}
