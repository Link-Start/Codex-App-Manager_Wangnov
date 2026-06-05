use serde::Serialize;

use crate::domain::manifest::MirrorEndpoints;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PayloadUpdateStatus {
    ReadyToCheck,
    Checking,
    UpdateAvailable,
    Current,
    Blocked,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayloadUpdateCheck {
    pub status: PayloadUpdateStatus,
    pub manifest_url: String,
    pub message: String,
}

impl PayloadUpdateCheck {
    pub fn pending(endpoints: &MirrorEndpoints) -> Self {
        Self {
            status: PayloadUpdateStatus::ReadyToCheck,
            manifest_url: endpoints.manifest_url.clone(),
            message: "Manifest client boundary is ready; network fetch and signature policy come next."
                .to_string(),
        }
    }
}

