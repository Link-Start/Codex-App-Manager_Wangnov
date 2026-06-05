use tauri::State;

use crate::app::health_service::HealthService;
use crate::app::snapshot::ManagerSnapshot;
use crate::app::update_check::PayloadUpdateCheck;
use crate::domain::health::HealthReport;
use crate::domain::operations::{OperationKind, OperationPlan};
use crate::app::mac_update::{
    plan_macos_update, stage_macos_update, MacInstallStatus, MacStageReport, MacUpdateReport,
    PROD_ARM64_APPCAST,
};
use crate::domain::target::OperatingSystem;
use crate::errors::{AppError, CommandError};
use crate::state::ManagerState;

#[tauri::command]
pub fn get_app_snapshot(state: State<'_, ManagerState>) -> Result<ManagerSnapshot, CommandError> {
    Ok(state.snapshot())
}

#[tauri::command]
pub fn plan_install(state: State<'_, ManagerState>) -> Result<OperationPlan, CommandError> {
    Ok(state.planner.plan(
        OperationKind::Install,
        &state.target,
        &state.settings,
        &state.endpoints,
    ))
}

#[tauri::command]
pub fn plan_uninstall(state: State<'_, ManagerState>) -> Result<OperationPlan, CommandError> {
    Ok(state.planner.plan(
        OperationKind::Uninstall,
        &state.target,
        &state.settings,
        &state.endpoints,
    ))
}

#[tauri::command]
pub fn check_payload_updates(
    state: State<'_, ManagerState>,
) -> Result<PayloadUpdateCheck, CommandError> {
    Ok(PayloadUpdateCheck::pending(&state.endpoints))
}

#[tauri::command]
pub fn run_health_check(state: State<'_, ManagerState>) -> Result<HealthReport, CommandError> {
    Ok(HealthService::run(&state.target, &state.settings, &state.endpoints))
}

/// macOS-only: detect the installed Codex build, read the Sparkle appcast, and
/// return an update plan (delta vs full). Read-only — performs no install.
#[tauri::command]
pub fn mac_plan_update(
    state: State<'_, ManagerState>,
    simulated_build: Option<u64>,
) -> Result<MacUpdateReport, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Macos) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    // TODO(mirror §4.4): point at the mirror's rewritten appcast, not oaistatic.
    plan_macos_update(PROD_ARM64_APPCAST, simulated_build).map_err(Into::into)
}

/// macOS-only: plan + download + size/EdDSA verify into staging. Non-destructive
/// (no apply/swap). Runs the blocking download off the main thread.
#[tauri::command]
pub async fn mac_stage_update(
    simulated_build: Option<u64>,
) -> Result<MacStageReport, CommandError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::UnsupportedPlatform.into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        stage_macos_update(PROD_ARM64_APPCAST, simulated_build)
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))?
    .map_err(Into::into)
}

/// macOS-only: classify the installed Codex (managed / external / none).
#[tauri::command]
pub fn mac_status(state: State<'_, ManagerState>) -> Result<MacInstallStatus, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Macos) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    Ok(crate::app::mac_update::mac_install_status())
}

/// macOS-only: adopt the detected external install (after explicit user consent).
#[tauri::command]
pub fn mac_adopt(state: State<'_, ManagerState>) -> Result<MacInstallStatus, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Macos) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    crate::app::mac_update::mac_adopt().map_err(Into::into)
}

