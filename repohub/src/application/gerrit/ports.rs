use crate::domain::{Approval, ApprovalRecord, Change, PatchSet, ReviewPolicy};
use std::{future::Future, pin::Pin};

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug)]
pub enum ReviewError {
    NotFound(String),
    InvalidInput(String),
    PolicyViolation(String),
    Storage(String),
}

#[derive(Debug, Clone)]
pub struct ChangeSummary {
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

impl std::fmt::Display for ReviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(message) => write!(f, "Not found: {message}"),
            Self::InvalidInput(message) => write!(f, "Invalid input: {message}"),
            Self::PolicyViolation(message) => write!(f, "Policy violation: {message}"),
            Self::Storage(message) => write!(f, "Storage error: {message}"),
        }
    }
}

impl std::error::Error for ReviewError {}

pub trait ChangeCommandPort {
    fn create_change<'a>(
        &'a self,
        change: Change,
        patch_set: PatchSet,
    ) -> PortFuture<'a, Result<Change, ReviewError>>;
    fn append_patch_set<'a>(
        &'a self,
        patch_set: PatchSet,
    ) -> PortFuture<'a, Result<(), ReviewError>>;
    fn save_vote<'a>(
        &'a self,
        change_id: i64,
        vote: ApprovalRecord,
    ) -> PortFuture<'a, Result<(), ReviewError>>;
    fn update_change_status<'a>(
        &'a self,
        change_id: i64,
        status: &'a str,
    ) -> PortFuture<'a, Result<(), ReviewError>>;
}

pub trait ChangeQueryPort {
    fn get_change<'a>(&'a self, change_id: i64) -> PortFuture<'a, Result<Change, ReviewError>>;
    fn list_changes_by_repository<'a>(
        &'a self,
        repository_id: i64,
    ) -> PortFuture<'a, Result<Vec<ChangeSummary>, ReviewError>>;
    fn list_approvals<'a>(
        &'a self,
        change_id: i64,
    ) -> PortFuture<'a, Result<Vec<ApprovalRecord>, ReviewError>>;
}

pub trait PolicyPort {
    fn get_policy_for_repository<'a>(
        &'a self,
        repository_id: i64,
    ) -> PortFuture<'a, Result<ReviewPolicy, ReviewError>>;
    fn validate_vote<'a>(
        &'a self,
        policy: &'a ReviewPolicy,
        approval: &'a Approval,
    ) -> PortFuture<'a, Result<(), ReviewError>>;
}
