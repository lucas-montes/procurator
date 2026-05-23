use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Normalized Commit model for cross-forge compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedCommit {
    /// Unique identifier for the commit (SHA)
    pub sha: String,
    /// Repository identifier
    pub repository_id: i64,
    /// Author of the commit
    pub author_id: i64,
    /// Committer of the commit (may be same as author)
    pub committer_id: i64,
    /// Commit message
    pub message: String,
    /// When the commit was authored (canonical coding-stage start timestamp)
    pub authored_at: DateTime<Utc>,
    /// When the commit was committed
    pub committed_at: DateTime<Utc>,
    /// Number of additions in the commit
    pub additions: u32,
    /// Number of deletions in the commit
    pub deletions: u32,
    /// Total number of changes (additions + deletions)
    pub total: u32,
}

impl NormalizedCommit {
    /// Calculate commit size (additions + deletions)
    pub fn size(&self) -> u32 {
        self.additions + self.deletions
    }

    /// Canonical commit timestamp for weekly commit metrics.
    pub fn committed_timestamp(&self) -> DateTime<Utc> {
        self.committed_at.clone()
    }

    /// Canonical authoring timestamp for coding-stage and lead-time metrics.
    pub fn coding_started_at(&self) -> DateTime<Utc> {
        self.authored_at.clone()
    }
}
