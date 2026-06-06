use serde::Serialize;

use crate::domain::installation::ManagedInstallation;
use crate::domain::manifest::MirrorEndpoints;
use crate::domain::settings::AppSettings;
use crate::domain::target::Target;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerSnapshot {
    pub manager_version: String,
    pub target: Target,
    pub settings: AppSettings,
    pub endpoints: MirrorEndpoints,
    pub installation: ManagedInstallation,
    pub available_actions: Vec<String>,
}
