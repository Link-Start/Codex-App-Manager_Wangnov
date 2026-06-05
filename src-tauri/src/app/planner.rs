use crate::domain::manifest::MirrorEndpoints;
use crate::domain::operations::{
    OperationKind, OperationPlan, OperationStep, OperationStrategy,
};
use crate::domain::settings::AppSettings;
use crate::domain::target::{OperatingSystem, Target};

#[derive(Debug, Clone, Default)]
pub struct InstallPlanner;

impl InstallPlanner {
    pub fn available_actions(&self, target: &Target) -> Vec<String> {
        match target.os {
            OperatingSystem::Windows | OperatingSystem::Macos => {
                vec!["install".to_string(), "update".to_string(), "uninstall".to_string()]
            }
            _ => vec!["inspect".to_string()],
        }
    }

    pub fn plan(
        &self,
        kind: OperationKind,
        target: &Target,
        settings: &AppSettings,
        endpoints: &MirrorEndpoints,
    ) -> OperationPlan {
        match (&target.os, &kind) {
            (OperatingSystem::Windows, OperationKind::Install)
            | (OperatingSystem::Windows, OperationKind::Update) => self.windows_install_plan(
                kind,
                settings,
                endpoints,
                OperationStrategy::WindowsMsixPreferred,
            ),
            (OperatingSystem::Macos, OperationKind::Install)
            | (OperatingSystem::Macos, OperationKind::Update) => {
                self.macos_install_plan(kind, settings, endpoints)
            }
            (OperatingSystem::Windows, OperationKind::Uninstall)
            | (OperatingSystem::Macos, OperationKind::Uninstall) => {
                self.uninstall_plan(kind, settings)
            }
            _ => OperationPlan {
                kind,
                strategy: OperationStrategy::Unsupported,
                install_root: settings.install_root.clone(),
                steps: vec![OperationStep::blocked(
                    "unsupported-platform",
                    "Unsupported platform",
                    "Only Windows and macOS payload management are planned for this app.",
                )],
            },
        }
    }

    fn windows_install_plan(
        &self,
        kind: OperationKind,
        settings: &AppSettings,
        endpoints: &MirrorEndpoints,
        strategy: OperationStrategy,
    ) -> OperationPlan {
        OperationPlan {
            kind,
            strategy,
            install_root: settings.install_root.clone(),
            steps: vec![
                OperationStep::ready(
                    "download-msix",
                    "Download official MSIX",
                    &endpoints.windows_msix_url,
                ),
                OperationStep::ready(
                    "verify-msix",
                    "Verify package hash and identity",
                    "Compare SHA256SUMS.txt and read AppxManifest.xml before install.",
                ),
                OperationStep::pending(
                    "install-msix",
                    "Install via Windows package path",
                    "Prefer App Installer/MSIX first; fixed-path unpacked install remains the fallback.",
                ),
                OperationStep::pending(
                    "register-state",
                    "Write manager state",
                    "Persist install provenance for future update and uninstall operations.",
                ),
            ],
        }
    }

    fn macos_install_plan(
        &self,
        kind: OperationKind,
        settings: &AppSettings,
        endpoints: &MirrorEndpoints,
    ) -> OperationPlan {
        OperationPlan {
            kind,
            strategy: OperationStrategy::MacosDmgReplace,
            install_root: settings.install_root.clone(),
            steps: vec![
                OperationStep::ready(
                    "download-dmg",
                    "Download official DMG",
                    &format!(
                        "arm64: {}; intel: {}",
                        endpoints.mac_arm64_url, endpoints.mac_intel_url
                    ),
                ),
                OperationStep::ready(
                    "verify-dmg",
                    "Verify image hash and code signature",
                    "Compare SHA256SUMS.txt, inspect bundle id, and verify Apple signing state.",
                ),
                OperationStep::pending(
                    "replace-app",
                    "Replace Codex.app",
                    "Mount the DMG, copy the app bundle to the managed install root, and keep a rollback copy.",
                ),
                OperationStep::pending(
                    "register-state",
                    "Write manager state",
                    "Persist install provenance for future update and uninstall operations.",
                ),
            ],
        }
    }

    fn uninstall_plan(&self, kind: OperationKind, settings: &AppSettings) -> OperationPlan {
        OperationPlan {
            kind,
            strategy: OperationStrategy::ManagedUninstall,
            install_root: settings.install_root.clone(),
            steps: vec![
                OperationStep::ready(
                    "stop-app",
                    "Stop Codex",
                    "Ask the user to close Codex before removing managed files.",
                ),
                OperationStep::pending(
                    "remove-managed-files",
                    "Remove managed install root",
                    "Delete only files known to be created by this manager.",
                ),
                OperationStep::pending(
                    "preserve-user-data",
                    "Preserve user data",
                    "User data stays unless the user explicitly selects a purge option.",
                ),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::target::{Architecture, Target};

    fn settings() -> AppSettings {
        AppSettings::new("https://codexapp.agentsmirror.com".to_string(), "root".to_string())
    }

    #[test]
    fn windows_install_prefers_msix() {
        let target = Target {
            os: OperatingSystem::Windows,
            arch: Architecture::X64,
            label: "Windows / X64".to_string(),
        };
        let plan = InstallPlanner::default().plan(
            OperationKind::Install,
            &target,
            &settings(),
            &MirrorEndpoints::from_base_url("https://example.test"),
        );

        assert!(matches!(plan.strategy, OperationStrategy::WindowsMsixPreferred));
        assert_eq!(plan.steps[0].id, "download-msix");
    }

    #[test]
    fn macos_install_uses_dmg_replace() {
        let target = Target {
            os: OperatingSystem::Macos,
            arch: Architecture::Arm64,
            label: "Macos / Arm64".to_string(),
        };
        let plan = InstallPlanner::default().plan(
            OperationKind::Install,
            &target,
            &settings(),
            &MirrorEndpoints::from_base_url("https://example.test"),
        );

        assert!(matches!(plan.strategy, OperationStrategy::MacosDmgReplace));
        assert_eq!(plan.steps[0].id, "download-dmg");
    }
}
