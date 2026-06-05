use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MirrorEndpoints {
    pub manifest_url: String,
    pub checksums_url: String,
    pub windows_msix_url: String,
    pub windows_unpacked_url: String,
    pub mac_arm64_url: String,
    pub mac_intel_url: String,
}

impl MirrorEndpoints {
    pub fn from_base_url(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/');

        Self {
            manifest_url: format!("{base}/latest/manifest"),
            checksums_url: format!("{base}/latest/checksums"),
            windows_msix_url: format!("{base}/latest/win"),
            windows_unpacked_url: format!("{base}/latest/win-unpacked"),
            mac_arm64_url: format!("{base}/latest/mac-arm64"),
            mac_intel_url: format!("{base}/latest/mac-intel"),
        }
    }
}

