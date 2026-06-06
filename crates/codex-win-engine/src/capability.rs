use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityState {
    Available,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCheck {
    pub state: CapabilityState,
    pub detail: String,
}

impl CapabilityCheck {
    pub fn available(detail: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::Available,
            detail: detail.into(),
        }
    }

    pub fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::Unavailable,
            detail: detail.into(),
        }
    }

    pub fn unknown(detail: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::Unknown,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SideloadRecommendation {
    MsixPreferred,
    PortableFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WinCapabilityReport {
    pub add_appx_package: CapabilityCheck,
    pub appx_service: CapabilityCheck,
    pub sideload_policy: CapabilityCheck,
    pub app_installer: CapabilityCheck,
    pub metered_network: CapabilityCheck,
    pub recommendation: SideloadRecommendation,
    pub notes: Vec<String>,
}

impl WinCapabilityReport {
    pub fn from_checks(
        add_appx_package: CapabilityCheck,
        appx_service: CapabilityCheck,
        sideload_policy: CapabilityCheck,
        app_installer: CapabilityCheck,
        metered_network: CapabilityCheck,
        mut notes: Vec<String>,
    ) -> Self {
        let msix_blocked = add_appx_package.state == CapabilityState::Unavailable
            || appx_service.state == CapabilityState::Unavailable
            || sideload_policy.state == CapabilityState::Unavailable;
        let recommendation = if msix_blocked {
            notes.push(
                "MSIX sideloading appears blocked; use the portable fallback when available."
                    .to_string(),
            );
            SideloadRecommendation::PortableFallback
        } else {
            if sideload_policy.state == CapabilityState::Unknown {
                notes.push(
                    "Sideload policy is not explicitly enabled; installation may still succeed on modern Windows and will fall back if it fails."
                        .to_string(),
                );
            }
            SideloadRecommendation::MsixPreferred
        };

        Self {
            add_appx_package,
            appx_service,
            sideload_policy,
            app_installer,
            metered_network,
            recommendation,
            notes,
        }
    }

    pub fn unknown_for_non_windows() -> Self {
        Self::from_checks(
            CapabilityCheck::unknown("not running on Windows"),
            CapabilityCheck::unknown("not running on Windows"),
            CapabilityCheck::unknown("not running on Windows"),
            CapabilityCheck::unknown("not running on Windows"),
            CapabilityCheck::unknown("not running on Windows"),
            vec!["Windows capability checks are only meaningful on Windows.".to_string()],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommends_msix_when_no_blocking_signal_exists() {
        let report = WinCapabilityReport::from_checks(
            CapabilityCheck::available("present"),
            CapabilityCheck::available("running"),
            CapabilityCheck::unknown("policy absent"),
            CapabilityCheck::available("installed"),
            CapabilityCheck::unknown("not probed"),
            vec![],
        );
        assert_eq!(report.recommendation, SideloadRecommendation::MsixPreferred);
    }

    #[test]
    fn recommends_portable_when_policy_blocks_sideloading() {
        let report = WinCapabilityReport::from_checks(
            CapabilityCheck::available("present"),
            CapabilityCheck::available("running"),
            CapabilityCheck::unavailable("AllowAllTrustedApps=0"),
            CapabilityCheck::available("installed"),
            CapabilityCheck::unknown("not probed"),
            vec![],
        );
        assert_eq!(
            report.recommendation,
            SideloadRecommendation::PortableFallback
        );
    }
}
