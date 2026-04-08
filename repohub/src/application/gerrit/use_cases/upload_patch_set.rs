use crate::{
    application::gerrit::{ChangeCommandPort, ChangeQueryPort, ReviewError},
    domain::PatchSet,
};

#[derive(Debug, Clone)]
pub struct UploadPatchSetInput {
    pub change_id: i64,
    pub revision: String,
    pub uploader_user_id: i64,
    pub kind: crate::domain::PatchSetKind,
}

pub struct UploadPatchSet<'handler, C, Q>
where
    C: ChangeCommandPort,
    Q: ChangeQueryPort,
{
    command_port: &'handler C,
    query_port: &'handler Q,
}

impl<'handler, C, Q> UploadPatchSet<'handler, C, Q>
where
    C: ChangeCommandPort,
    Q: ChangeQueryPort,
{
    pub fn new(command_port: &'handler C, query_port: &'handler Q) -> Self {
        Self {
            command_port,
            query_port,
        }
    }

    pub async fn execute(&self, input: UploadPatchSetInput) -> Result<(), ReviewError> {
        let change = self.query_port.get_change(input.change_id).await?;

        let patch_set = PatchSet {
            change_id: input.change_id,
            number: change.current_patch_set + 1,
            revision: input.revision,
            kind: input.kind,
            uploader_user_id: input.uploader_user_id,
        };

        self.command_port.append_patch_set(patch_set).await
    }
}
