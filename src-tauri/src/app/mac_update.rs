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

use crate::app::provenance::ProvenanceStore;
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

/// arm64 / x64 Sparkle appcasts, served by the mirror (CN-reachable; enclosure
/// URLs rewritten to R2/S3). EdDSA signatures are preserved (they sign bytes,
/// not URLs), so the pinned-key verification still passes against mirrored files.
pub const PROD_ARM64_APPCAST: &str = "https://codexapp.agentsmirror.com/latest/appcast.xml";
pub const PROD_X64_APPCAST: &str = "https://codexapp.agentsmirror.com/latest/appcast-x64.xml";

/// The mirror appcast matching the host architecture — you manage the Codex for
/// the machine you are on.
pub fn host_appcast() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => PROD_X64_APPCAST,
        _ => PROD_ARM64_APPCAST,
    }
}

fn parse_macos_version(s: &str) -> Option<(u32, u32)> {
    let mut parts = s.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|m| m.parse().ok()).unwrap_or(0);
    Some((major, minor))
}

fn host_macos_version() -> Option<(u32, u32)> {
    let out = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_macos_version(&String::from_utf8_lossy(&out.stdout))
}

/// Reject staging/installing an update the host macOS is too old to run. If the
/// requirement or host version can't be parsed, we don't block.
fn require_os_supported(required: Option<&str>) -> Result<(), AppError> {
    let (Some(req), Some(host)) = (
        required.and_then(parse_macos_version),
        host_macos_version(),
    ) else {
        return Ok(());
    };
    if host >= req {
        Ok(())
    } else {
        Err(AppError::Engine(format!(
            "this macOS ({}.{}) is older than the latest Codex requires ({}.{}+)",
            host.0, host.1, req.0, req.1
        )))
    }
}

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

    if let Some(latest) = appcast.latest() {
        require_os_supported(latest.minimum_system_version.as_deref())?;
    }

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

    if let Some(latest) = appcast.latest() {
        require_os_supported(latest.minimum_system_version.as_deref())?;
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacInstallStatus {
    pub installed: Option<InstalledCodex>,
    /// "managed" | "external" | "none"
    pub status: String,
}

/// Classify the installed Codex against our provenance store.
pub fn mac_install_status() -> MacInstallStatus {
    let installed = detect_installed();
    let store = ProvenanceStore::load();
    let status = match &installed {
        None => "none",
        Some(codex) if store.is_managed(&codex.path) => "managed",
        Some(_) => "external",
    }
    .to_string();
    MacInstallStatus { installed, status }
}

/// Adopt the detected install — record provenance after explicit user consent.
pub fn mac_adopt() -> Result<MacInstallStatus, AppError> {
    let installed = detect_installed()
        .ok_or_else(|| AppError::Internal("no Codex detected to adopt".to_string()))?;
    let mut store = ProvenanceStore::load();
    store.record(installed.path.clone(), installed.build, "adopted-external");
    store.save()?;
    Ok(mac_install_status())
}
