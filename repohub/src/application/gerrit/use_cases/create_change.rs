use crate::{
    application::gerrit::{ChangeCommandPort, ReviewError},
    domain::{Change, ChangeStatus, PatchSet, PatchSetKind},
};

#[derive(Debug, Clone)]
pub struct CreateChangeInput {
    pub repository_id: i64,
    pub change_key: String,
    pub target_branch: String,
    pub subject: String,
    pub owner_user_id: i64,
    pub revision: String,
    pub kind: PatchSetKind,
}

pub struct CreateChange<'handler, C>
where
    C: ChangeCommandPort,
{
    command_port: &'handler C,
}

impl<'handler, C> CreateChange<'handler, C>
where
    C: ChangeCommandPort,
{
    pub fn new(command_port: &'handler C) -> Self {
        Self { command_port }
    }

    pub async fn execute(&self, input: CreateChangeInput) -> Result<Change, ReviewError> {
        let change = Change {
            id: 0,
            repository_id: input.repository_id,
            change_key: input.change_key,
            target_branch: input.target_branch,
            subject: input.subject,
            owner_user_id: input.owner_user_id,
            status: ChangeStatus::New,
            current_patch_set: 1,
        };

        let patch_set = PatchSet {
            change_id: change.id,
            number: 1,
            revision: input.revision,
            kind: input.kind,
            uploader_user_id: change.owner_user_id,
        };

        self.command_port.create_change(change, patch_set).await
    }
}
