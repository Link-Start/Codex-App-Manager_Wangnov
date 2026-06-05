use crate::domain::health::{HealthCheck, HealthReport, HealthStatus};
use crate::domain::manifest::MirrorEndpoints;
use crate::domain::settings::AppSettings;
use crate::domain::target::{OperatingSystem, Target};

pub struct HealthService;

impl HealthService {
    pub fn run(target: &Target, settings: &AppSettings, endpoints: &MirrorEndpoints) -> HealthReport {
        let mut checks = vec![
            HealthCheck {
                id: "platform".to_string(),
                label: "Platform adapter".to_string(),
                status: match target.os {
                    OperatingSystem::Windows | OperatingSystem::Macos => HealthStatus::Ok,
                    _ => HealthStatus::Warning,
                },
                detail: target.label.clone(),
            },
            HealthCheck {
                id: "install-root".to_string(),
                label: "Install root".to_string(),
                status: HealthStatus::Ok,
                detail: settings.install_root.clone(),
            },
            HealthCheck {
                id: "manifest".to_string(),
                label: "Mirror manifest".to_string(),
                status: HealthStatus::Ok,
                detail: endpoints.manifest_url.clone(),
            },
        ];

        if !settings.preserve_user_data_by_default {
            checks.push(HealthCheck {
                id: "user-data".to_string(),
                label: "User data policy".to_string(),
                status: HealthStatus::Warning,
                detail: "User data is not preserved by default.".to_string(),
            });
        }

        HealthReport { checks }
    }
}

