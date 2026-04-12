use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CreateChangeRequest {
    pub subject: String,
    pub target_branch: String,
    pub revision: String,
    pub change_key: Option<String>,
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UploadPatchSetRequest {
    pub revision: String,
    pub uploader_username: Option<String>,
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VoteRequest {
    pub reviewer_username: String,
    pub label: String,
    pub value: i32,
}

#[derive(Debug, Serialize, Clone)]
pub struct ChangeDto {
    pub id: i64,
    pub repository_id: i64,
    pub change_key: String,
    pub target_branch: String,
    pub subject: String,
    pub owner_user_id: i64,
    pub status: String,
    pub current_patch_set: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct ApprovalDto {
    pub user_id: i64,
    pub label: String,
    pub value: i32,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReadinessCheckDto {
    pub name: String,
    pub passed: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct ChangeDetailDto {
    pub change: ChangeDto,
    pub approvals: Vec<ApprovalDto>,
    pub readiness_ready: bool,
    pub readiness_checks: Vec<ReadinessCheckDto>,
}
