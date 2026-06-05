//! macOS update planning + staging service.
//!
//! Bridges the pure `codex-mac-engine` (detect + appcast + plan + download +
//! verify) to the Tauri command surface.
//!
//! - `plan_macos_update`  — read-only: what would we download? (delta vs full)
//! - `stage_macos_update` — download the artifact + size + EdDSA verify into a
//!   staging dir. Non-destructive: it does NOT apply/swap. The destructive tail
//!   (BinaryDelta apply → codesign gate → atomic swap → relaunch) comes next.
//!
//! `simulated_build` lets the UI preview the delta path even when the machine is
//! already on the latest build (the user's case during development).

use serde::Serialize;

use codex_mac_engine::{
    download, parse_appcast, plan_update, sys, verify_sparkle, UpdatePlan, UpdateStrategy,
};

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
    pub simulated_build: Option<u64>,
    pub plan: Option<UpdatePlan>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacStageReport {
    pub up_to_date: bool,
    pub strategy: String,
    pub latest_build: u64,
    pub latest_short_version: String,
    pub download_size: u64,
    pub full_size: u64,
    pub savings_pct: f64,
    pub staged_path: Option<String>,
    /// EdDSA signature verified against the pinned Sparkle key.
    pub verified: bool,
}

/// Prod arm64 Sparkle appcast.
///
/// TODO(§4.4): replace with the mirror's rewritten, CN-reachable appcast once
/// the mirror publishes one (enclosure URLs pointed at R2/S3).
pub const PROD_ARM64_APPCAST: &str =
    "https://persistent.oaistatic.com/codex-app-prod/appcast.xml";

fn effective_build(simulated_build: Option<u64>, installed: &Option<InstalledCodex>) -> u64 {
    simulated_build
        .or_else(|| installed.as_ref().map(|i| i.build))
        .unwrap_or(0)
}

fn detect_installed() -> Option<InstalledCodex> {
    sys::installed_codex_build().map(|(path, build)| InstalledCodex { path, build })
}

pub fn plan_macos_update(
    appcast_url: &str,
    simulated_build: Option<u64>,
) -> Result<MacUpdateReport, AppError> {
    let installed = detect_installed();
    let xml = sys::fetch_text(appcast_url).map_err(|e| AppError::Engine(e.to_string()))?;
    let appcast = parse_appcast(&xml).map_err(|e| AppError::Engine(e.to_string()))?;
    let plan = plan_update(&appcast, effective_build(simulated_build, &installed));

    Ok(MacUpdateReport {
        appcast_url: appcast_url.to_string(),
        installed,
        simulated_build,
        plan,
    })
}

fn staging_dir() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("codex-app-manager")
        .join("staging")
}

fn strategy_label(strategy: &UpdateStrategy) -> String {
    match strategy {
        UpdateStrategy::Delta { from_build } => format!("delta-from-{from_build}"),
        UpdateStrategy::Full => "full".to_string(),
    }
}

pub fn stage_macos_update(
    appcast_url: &str,
    simulated_build: Option<u64>,
) -> Result<MacStageReport, AppError> {
    let installed = detect_installed();
    let xml = sys::fetch_text(appcast_url).map_err(|e| AppError::Engine(e.to_string()))?;
    let appcast = parse_appcast(&xml).map_err(|e| AppError::Engine(e.to_string()))?;
    let plan = plan_update(&appcast, effective_build(simulated_build, &installed))
        .ok_or_else(|| AppError::Engine("appcast had no items".to_string()))?;

    if plan.up_to_date {
        return Ok(MacStageReport {
            up_to_date: true,
            strategy: "none".to_string(),
            latest_build: plan.latest_build,
            latest_short_version: plan.latest_short_version,
            download_size: 0,
            full_size: plan.full_size,
            savings_pct: 0.0,
            staged_path: None,
            verified: false,
        });
    }

    let signature = plan
        .ed_signature
        .clone()
        .ok_or_else(|| AppError::Engine("appcast enclosure missing edSignature".to_string()))?;

    let file_name = plan
        .download_url
        .rsplit('/')
        .next()
        .unwrap_or("payload.bin");
    let dest = staging_dir().join(file_name);

    // Reuse an already-staged artifact of the right size (idempotent).
    let already = std::fs::metadata(&dest)
        .map(|m| m.len() == plan.download_size)
        .unwrap_or(false);
    if !already {
        download::download_to(&plan.download_url, &dest)
            .map_err(|e| AppError::Engine(e.to_string()))?;
    }

    // Size gate.
    let len = std::fs::metadata(&dest)
        .map_err(|e| AppError::Engine(e.to_string()))?
        .len();
    if len != plan.download_size {
        return Err(AppError::Engine(format!(
            "size mismatch: {len} != {}",
            plan.download_size
        )));
    }

    // EdDSA gate (pinned key).
    let bytes = download::read_file(&dest).map_err(|e| AppError::Engine(e.to_string()))?;
    verify_sparkle(&bytes, &signature).map_err(|e| AppError::Engine(e.to_string()))?;

    Ok(MacStageReport {
        up_to_date: false,
        strategy: strategy_label(&plan.strategy),
        latest_build: plan.latest_build,
        latest_short_version: plan.latest_short_version,
        download_size: plan.download_size,
        full_size: plan.full_size,
        savings_pct: plan.savings_pct,
        staged_path: Some(dest.to_string_lossy().into_owned()),
        verified: true,
    })
}
