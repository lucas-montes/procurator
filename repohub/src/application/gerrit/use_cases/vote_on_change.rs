use crate::{
    application::gerrit::{ChangeCommandPort, PolicyPort, ReviewError},
    domain::{Approval, ApprovalRecord},
};

#[derive(Debug, Clone)]
pub struct VoteOnChangeInput {
    pub change_id: i64,
    pub repository_id: i64,
    pub user_id: i64,
    pub label: String,
    pub value: i32,
}

pub struct VoteOnChange<'handler, C, P>
where
    C: ChangeCommandPort,
    P: PolicyPort,
{
    command_port: &'handler C,
    policy_port: &'handler P,
}

impl<'handler, C, P> VoteOnChange<'handler, C, P>
where
    C: ChangeCommandPort,
    P: PolicyPort,
{
    pub fn new(command_port: &'handler C, policy_port: &'handler P) -> Self {
        Self {
            command_port,
            policy_port,
        }
    }

    pub async fn execute(&self, input: VoteOnChangeInput) -> Result<(), ReviewError> {
        let policy = self
            .policy_port
            .get_policy_for_repository(input.repository_id)
            .await?;

        let approval = Approval {
            label: input.label,
            value: input.value,
        };

        self.policy_port.validate_vote(&policy, &approval).await?;

        let record = ApprovalRecord {
            user_id: input.user_id,
            approval,
        };

        self.command_port.save_vote(input.change_id, record).await
    }
}
