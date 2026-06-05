use tauri::State;

use crate::app::health_service::HealthService;
use crate::app::snapshot::ManagerSnapshot;
use crate::app::update_check::PayloadUpdateCheck;
use crate::domain::health::HealthReport;
use crate::domain::operations::{OperationKind, OperationPlan};
use crate::app::mac_update::{plan_macos_update, MacUpdateReport, PROD_ARM64_APPCAST};
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
pub fn mac_plan_update(state: State<'_, ManagerState>) -> Result<MacUpdateReport, CommandError> {
    if !matches!(state.target.os, OperatingSystem::Macos) {
        return Err(AppError::UnsupportedPlatform.into());
    }
    // TODO(mirror §4.4): point at the mirror's rewritten appcast, not oaistatic.
    plan_macos_update(PROD_ARM64_APPCAST).map_err(Into::into)
}

