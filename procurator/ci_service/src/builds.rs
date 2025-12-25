use serde::{Deserialize, Serialize};

#[derive(Debug, sqlx::FromRow)]
pub struct BuildJob {
    id: i64,
    repo_path: String,
    commit_hash: String,
    branch: String,
    status: String,
    retry_count: u8,
    max_retries: u8,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
}

impl BuildJob {
    pub fn id(&self) -> i64 {
        self.id
    }

    pub fn git_url(&self) -> String {
        format!("{}#{}", self.repo_path, self.commit_hash)
    }

    pub fn can_retry(&self) -> bool {
        self.retry_count < self.max_retries
    }
}

#[derive(Debug, Serialize)]
pub struct BuildInfo {
    id: i64,
    repo_path: String,
    commit_hash: String,
    branch: String,
    status: BuildStatus,
    retry_count: u8,
}

impl From<BuildJob> for BuildInfo {
    fn from(b: BuildJob) -> Self {
        let status = b.status.parse().unwrap_or(BuildStatus::Queued);
        Self {
            id: b.id,
            repo_path: b.repo_path,
            commit_hash: b.commit_hash,
            branch: b.branch,
            status,
            retry_count: b.retry_count as u8,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
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
