use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallationStatus {
    NotDetected,
    Managed,
    External,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedInstallation {
    pub status: InstallationStatus,
    pub install_root: String,
    pub detected_version: Option<String>,
    pub managed_by_this_app: bool,
}

impl ManagedInstallation {
    pub fn not_detected(install_root: String) -> Self {
        Self {
            status: InstallationStatus::NotDetected,
            install_root,
            detected_version: None,
            managed_by_this_app: false,
        }
    }
}

