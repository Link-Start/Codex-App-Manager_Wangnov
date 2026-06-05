use tauri::State;

use crate::app::health_service::HealthService;
use crate::app::snapshot::ManagerSnapshot;
use crate::app::update_check::PayloadUpdateCheck;
use crate::domain::health::HealthReport;
use crate::domain::operations::{OperationKind, OperationPlan};
use crate::errors::CommandError;
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

