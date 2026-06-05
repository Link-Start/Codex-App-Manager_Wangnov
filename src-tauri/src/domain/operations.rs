use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OperationKind {
    Install,
    Update,
    Uninstall,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationStrategy {
    WindowsMsixPreferred,
    WindowsFixedPathUnpacked,
    MacosDmgReplace,
    ManagedUninstall,
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationStepStatus {
    Ready,
    Pending,
    Blocked,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationStep {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub status: OperationStepStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationPlan {
    pub kind: OperationKind,
    pub strategy: OperationStrategy,
    pub install_root: String,
    pub steps: Vec<OperationStep>,
}

impl OperationStep {
    pub fn ready(id: &str, title: &str, detail: &str) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            detail: detail.to_string(),
            status: OperationStepStatus::Ready,
        }
    }

    pub fn pending(id: &str, title: &str, detail: &str) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            detail: detail.to_string(),
            status: OperationStepStatus::Pending,
        }
    }

    pub fn blocked(id: &str, title: &str, detail: &str) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            detail: detail.to_string(),
            status: OperationStepStatus::Blocked,
        }
    }
}
