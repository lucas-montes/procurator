
use serde::{Deserialize, Serialize};


/// Domain model for a CI build
///
/// This is the core business entity that represents a build job.
/// It contains all information needed for build execution and has no
/// dependencies on infrastructure concerns.
#[derive(Debug, Clone)]
pub struct Build {
    id: BuildId,
    repository: RepositoryInfo,
    commit: CommitInfo,
    status: BuildStatus,
    retry: RetryInfo,
    timestamps: Timestamps,
}

impl Build {
    #[allow(dead_code)]
    pub fn new(
        id: BuildId,
        repository: RepositoryInfo,
        commit: CommitInfo,
        status: BuildStatus,
        retry: RetryInfo,
        timestamps: Timestamps,
    ) -> Self {
        Self {
            id,
            repository,
            commit,
            status,
            retry,
            timestamps,
        }
    }

    pub fn can_retry(&self) -> bool {
        self.retry.can_retry()
    }

    pub fn id(&self) -> i64 {
        self.id.0
    }

    pub fn username(&self) -> &str {
        &self.repository.username
    }

    pub fn repo_name(&self) -> &str {
        &self.repository.name
    }

    pub fn commit_hash(&self) -> &str {
        &self.commit.hash
    }

    pub fn branch(&self) -> &str {
        &self.commit.branch
    }

    pub fn status(&self) -> BuildStatus {
        self.status
    }

    pub fn retry_count(&self) -> i64 {
        self.retry.count
    }

    pub fn max_retries(&self) -> i64 {
        self.retry.max
    }

    pub fn created_at(&self) -> &str {
        &self.timestamps.created_at
    }

    pub fn started_at(&self) -> Option<&str> {
        self.timestamps.started_at.as_deref()
    }

    pub fn finished_at(&self) -> Option<&str> {
        self.timestamps.finished_at.as_deref()
    }
}

// ============================================================================
// Value Objects
// ============================================================================

#[derive(Debug, Clone, Copy)]
pub struct BuildId(pub i64);

/// Repository information needed for build execution
#[derive(Debug, Clone)]
pub struct RepositoryInfo {
    pub username: String,
    pub name: String,
}

impl RepositoryInfo {
    pub fn new(username: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            name: name.into(),
        }
    }
}

/// Commit information for the build
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub hash: String,
    pub branch: String,
}

impl CommitInfo {
    pub fn new(hash: impl Into<String>, branch: impl Into<String>) -> Self {
        Self {
            hash: hash.into(),
            branch: branch.into(),
        }
    }
}

/// Retry logic encapsulation
#[derive(Debug, Clone)]
pub struct RetryInfo {
    pub count: i64,
    pub max: i64,
}

impl RetryInfo {
    pub fn new(count: i64, max: i64) -> Self {
        Self { count, max }
    }

    pub fn can_retry(&self) -> bool {
        self.count < self.max
    }
}

/// Build timestamps
#[derive(Debug, Clone)]
pub struct Timestamps {
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

impl Timestamps {
    pub fn new(
        created_at: impl Into<String>,
        started_at: Option<String>,
        finished_at: Option<String>,
    ) -> Self {
        Self {
            created_at: created_at.into(),
            started_at,
            finished_at,
        }
    }
}

// ============================================================================
// Build Status
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildStatus {
    Queued,
    Running,
    Success,
    Failed,
}

impl BuildStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BuildStatus::Queued => "queued",
            BuildStatus::Running => "running",
            BuildStatus::Success => "success",
            BuildStatus::Failed => "failed",
        }
    }
}

impl std::fmt::Display for BuildStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for BuildStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "queued" => Ok(BuildStatus::Queued),
            "running" => Ok(BuildStatus::Running),
            "success" => Ok(BuildStatus::Success),
            "failed" => Ok(BuildStatus::Failed),
            _ => Err(format!("Invalid build status: {}", s)),
        }
    }
}

impl TryFrom<String> for BuildStatus {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
