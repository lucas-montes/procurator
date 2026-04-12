use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChangeStatus {
    New,
    Merged,
    Abandoned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PatchSetKind {
    RefUpload,
    WebUpload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub id: i64,
    pub repository_id: i64,
    pub change_key: String,
    pub target_branch: String,
    pub subject: String,
    pub owner_user_id: i64,
    pub status: ChangeStatus,
    pub current_patch_set: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchSet {
    pub change_id: i64,
    pub number: i32,
    pub revision: String,
    pub kind: PatchSetKind,
    pub uploader_user_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelDefinition {
    pub name: String,
    pub min: i32,
    pub max: i32,
    pub default: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitRequirement {
    pub label: String,
    pub min_value: i32,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GerritSubmitType {
    RebaseIfNecessary,
    FastForwardOnly,
    MergeIfNecessary,
    CherryPick,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewPolicy {
    pub labels: Vec<LabelDefinition>,
    pub requirements: Vec<SubmitRequirement>,
    pub submit_type: GerritSubmitType,
}

impl ReviewPolicy {
    pub fn gerrit_default() -> Self {
        Self {
            labels: vec![
                LabelDefinition {
                    name: "Code-Review".to_string(),
                    min: -2,
                    max: 2,
                    default: 0,
                },
                LabelDefinition {
                    name: "Verified".to_string(),
                    min: -1,
                    max: 1,
                    default: 0,
                },
            ],
            requirements: vec![
                SubmitRequirement {
                    label: "Code-Review".to_string(),
                    min_value: 2,
                    required: true,
                },
                SubmitRequirement {
                    label: "Verified".to_string(),
                    min_value: 1,
                    required: true,
                },
            ],
            submit_type: GerritSubmitType::RebaseIfNecessary,
        }
    }

    pub fn label_definition(&self, name: &str) -> Option<&LabelDefinition> {
        self.labels
            .iter()
            .find(|definition| definition.name == name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub label: String,
    pub value: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub user_id: i64,
    pub approval: Approval,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitReadiness {
    pub ready: bool,
    pub checks: BTreeMap<String, bool>,
}

impl SubmitReadiness {
    pub fn evaluate(
        policy: &ReviewPolicy,
        approvals: &[ApprovalRecord],
        verified_ok: bool,
    ) -> Self {
        let mut maxima: HashMap<&str, i32> = HashMap::new();
        for record in approvals {
            let current = maxima
                .entry(record.approval.label.as_str())
                .or_insert(record.approval.value);
            if record.approval.value > *current {
                *current = record.approval.value;
            }
        }

        let mut checks = BTreeMap::new();

        for requirement in &policy.requirements {
            if !requirement.required {
                continue;
            }

            let value = maxima.get(requirement.label.as_str()).copied().unwrap_or(0);
            checks.insert(requirement.label.clone(), value >= requirement.min_value);
        }

        checks.insert("Verified-Integration".to_string(), verified_ok);

        let ready = checks.values().all(|result| *result);
        Self { ready, checks }
    }
}

#[cfg(test)]
mod tests {
    use super::{Approval, ApprovalRecord, ReviewPolicy, SubmitReadiness};

    #[test]
    fn gerrit_default_has_expected_labels_and_submit_type() {
        let policy = ReviewPolicy::gerrit_default();

        assert!(policy.label_definition("Code-Review").is_some());
        assert!(policy.label_definition("Verified").is_some());
    }

    #[test]
    fn submit_readiness_passes_with_required_votes() {
        let policy = ReviewPolicy::gerrit_default();
        let approvals = vec![
            ApprovalRecord {
                user_id: 1,
                approval: Approval {
                    label: "Code-Review".to_string(),
                    value: 2,
                },
            },
            ApprovalRecord {
                user_id: 2,
                approval: Approval {
                    label: "Verified".to_string(),
                    value: 1,
                },
            },
        ];

        let readiness = SubmitReadiness::evaluate(&policy, &approvals, true);
        assert!(readiness.ready);
    }
}
