//! macOS update planning service (read-only slice).
//!
//! Bridges the pure `codex-mac-engine` (detect + appcast + plan) to the Tauri
//! command surface. No install / replace happens here yet — see docs §6 for the
//! full delta engine flow this will grow into.

use serde::Serialize;

use codex_mac_engine::{parse_appcast, plan_update, sys, UpdatePlan};

use crate::errors::AppError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledCodex {
    pub path: String,
    pub build: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacUpdateReport {
    pub appcast_url: String,
    pub installed: Option<InstalledCodex>,
    pub plan: Option<UpdatePlan>,
}

/// Prod arm64 Sparkle appcast.
///
/// TODO(§4.4): replace with the mirror's rewritten, CN-reachable appcast once
/// the mirror publishes one (enclosure URLs pointed at R2/S3).
pub const PROD_ARM64_APPCAST: &str =
    "https://persistent.oaistatic.com/codex-app-prod/appcast.xml";

pub fn plan_macos_update(appcast_url: &str) -> Result<MacUpdateReport, AppError> {
    let installed =
        sys::installed_codex_build().map(|(path, build)| InstalledCodex { path, build });

    let xml = sys::fetch_text(appcast_url).map_err(|e| AppError::Engine(e.to_string()))?;
    let appcast = parse_appcast(&xml).map_err(|e| AppError::Engine(e.to_string()))?;

    // No install detected → plan from build 0 (yields a Full "fresh install" basis).
    let current = installed.as_ref().map(|i| i.build).unwrap_or(0);
    let plan = plan_update(&appcast, current);

    Ok(MacUpdateReport {
        appcast_url: appcast_url.to_string(),
        installed,
        plan,
    })
}
