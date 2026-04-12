use crate::{
    adapters::shared::database::Database,
    application::gerrit::{
        ChangeCommandPort, ChangeQueryPort, ChangeSummary, PolicyPort, PortFuture, ReviewError,
    },
    domain::{
        Approval, ApprovalRecord, Change, ChangeStatus, PatchSet, PatchSetKind, ReviewPolicy,
    },
};

#[derive(Clone)]
pub struct SqliteReviewRepository {
    pub db: Database,
}

impl SqliteReviewRepository {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    fn map_db_error(error: crate::adapters::shared::database::DatabaseError) -> ReviewError {
        match error {
            crate::adapters::shared::database::DatabaseError::NotFound(message) => {
                ReviewError::NotFound(message)
            }
            other => ReviewError::Storage(other.to_string()),
        }
    }

    fn parse_change_status(value: &str) -> Result<ChangeStatus, ReviewError> {
        match value {
            "New" => Ok(ChangeStatus::New),
            "Merged" => Ok(ChangeStatus::Merged),
            "Abandoned" => Ok(ChangeStatus::Abandoned),
            _ => Err(ReviewError::Storage(format!(
                "Unknown review change status '{}'",
                value
            ))),
        }
    }

    fn patch_set_kind_as_str(kind: &PatchSetKind) -> &'static str {
        match kind {
            PatchSetKind::RefUpload => "RefUpload",
            PatchSetKind::WebUpload => "WebUpload",
        }
    }
}

impl ChangeCommandPort for SqliteReviewRepository {
    fn create_change<'a>(
        &'a self,
        change: Change,
        patch_set: PatchSet,
    ) -> PortFuture<'a, Result<Change, ReviewError>> {
        Box::pin(async move {
            let status = match change.status {
                ChangeStatus::New => "New",
                ChangeStatus::Merged => "Merged",
                ChangeStatus::Abandoned => "Abandoned",
            };

            let change_id = self
                .db
                .create_review_change(
                    change.repository_id,
                    &change.change_key,
                    &change.target_branch,
                    &change.subject,
                    change.owner_user_id,
                    status,
                    change.current_patch_set,
                )
                .await
                .map_err(Self::map_db_error)?;

            self.db
                .append_review_patch_set(
                    change_id,
                    patch_set.number,
                    &patch_set.revision,
                    Self::patch_set_kind_as_str(&patch_set.kind),
                    patch_set.uploader_user_id,
                )
                .await
                .map_err(Self::map_db_error)?;

            Ok(Change {
                id: change_id,
                ..change
            })
        })
    }

    fn append_patch_set<'a>(
        &'a self,
        patch_set: PatchSet,
    ) -> PortFuture<'a, Result<(), ReviewError>> {
        Box::pin(async move {
            self.db
                .append_review_patch_set(
                    patch_set.change_id,
                    patch_set.number,
                    &patch_set.revision,
                    Self::patch_set_kind_as_str(&patch_set.kind),
                    patch_set.uploader_user_id,
                )
                .await
                .map_err(Self::map_db_error)
        })
    }

    fn save_vote<'a>(
        &'a self,
        change_id: i64,
        vote: ApprovalRecord,
    ) -> PortFuture<'a, Result<(), ReviewError>> {
        Box::pin(async move {
            self.db
                .upsert_review_approval(
                    change_id,
                    vote.user_id,
                    &vote.approval.label,
                    vote.approval.value,
                )
                .await
                .map_err(Self::map_db_error)
        })
    }

    fn update_change_status<'a>(
        &'a self,
        change_id: i64,
        status: &'a str,
    ) -> PortFuture<'a, Result<(), ReviewError>> {
        Box::pin(async move {
            self.db
                .update_review_change_status(change_id, status)
                .await
                .map_err(Self::map_db_error)
        })
    }
}

impl ChangeQueryPort for SqliteReviewRepository {
    fn get_change<'a>(&'a self, change_id: i64) -> PortFuture<'a, Result<Change, ReviewError>> {
        Box::pin(async move {
            let row = self
                .db
                .get_review_change(change_id)
                .await
                .map_err(Self::map_db_error)?;

            Ok(Change {
                id: row.id,
                repository_id: row.repository_id,
                change_key: row.change_key,
                target_branch: row.target_branch,
                subject: row.subject,
                owner_user_id: row.owner_user_id,
                status: Self::parse_change_status(&row.status)?,
                current_patch_set: row.current_patch_set,
            })
        })
    }

    fn list_changes_by_repository<'a>(
        &'a self,
        repository_id: i64,
    ) -> PortFuture<'a, Result<Vec<ChangeSummary>, ReviewError>> {
        Box::pin(async move {
            let rows = self
                .db
                .list_review_changes_by_repository(repository_id)
                .await
                .map_err(Self::map_db_error)?;

            Ok(rows
                .into_iter()
                .map(|row| ChangeSummary {
                    id: row.id,
                    repository_id: row.repository_id,
                    change_key: row.change_key,
                    target_branch: row.target_branch,
                    subject: row.subject,
                    owner_user_id: row.owner_user_id,
                    status: row.status,
                    current_patch_set: row.current_patch_set,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                })
                .collect())
        })
    }

    fn list_approvals<'a>(
        &'a self,
        change_id: i64,
    ) -> PortFuture<'a, Result<Vec<ApprovalRecord>, ReviewError>> {
        Box::pin(async move {
            let rows = self
                .db
                .list_review_approvals(change_id)
                .await
                .map_err(Self::map_db_error)?;

            let approvals = rows
                .into_iter()
                .map(|row| ApprovalRecord {
                    user_id: row.user_id,
                    approval: Approval {
                        label: row.label,
                        value: row.value,
                    },
                })
                .collect();

            Ok(approvals)
        })
    }
}

impl PolicyPort for SqliteReviewRepository {
    fn get_policy_for_repository<'a>(
        &'a self,
        repository_id: i64,
    ) -> PortFuture<'a, Result<ReviewPolicy, ReviewError>> {
        Box::pin(async move {
            let repo = self
                .db
                .get_repository_by_id(repository_id)
                .await
                .map_err(Self::map_db_error)?;

            if let Some(repository_policy) = self
                .db
                .get_review_policy_override("repository", repository_id)
                .await
                .map_err(Self::map_db_error)?
            {
                return serde_json::from_str::<ReviewPolicy>(&repository_policy.policy_json)
                    .map_err(|error| ReviewError::Storage(error.to_string()));
            }

            if let Some(project_policy) = self
                .db
                .get_review_policy_override("project", repo.project_id)
                .await
                .map_err(Self::map_db_error)?
            {
                return serde_json::from_str::<ReviewPolicy>(&project_policy.policy_json)
                    .map_err(|error| ReviewError::Storage(error.to_string()));
            }

            if let Some(global_policy) = self
                .db
                .get_review_policy_override("global", -1)
                .await
                .map_err(Self::map_db_error)?
            {
                return serde_json::from_str::<ReviewPolicy>(&global_policy.policy_json)
                    .map_err(|error| ReviewError::Storage(error.to_string()));
            }

            Ok(ReviewPolicy::gerrit_default())
        })
    }

    fn validate_vote<'a>(
        &'a self,
        policy: &'a ReviewPolicy,
        approval: &'a Approval,
    ) -> PortFuture<'a, Result<(), ReviewError>> {
        Box::pin(async move {
            let definition = policy.label_definition(&approval.label).ok_or_else(|| {
                ReviewError::PolicyViolation(format!("Unknown label: {}", approval.label))
            })?;

            if approval.value < definition.min || approval.value > definition.max {
                return Err(ReviewError::PolicyViolation(format!(
                    "Vote {} for label {} is out of range [{}..{}]",
                    approval.value, approval.label, definition.min, definition.max
                )));
            }

            Ok(())
        })
    }
}
