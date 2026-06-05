pub mod adapters;
pub mod app;
pub mod commands;
pub mod domain;
pub mod errors;
pub mod ports;

mod state;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(state::ManagerState::new())
        .invoke_handler(tauri::generate_handler![
            commands::check_payload_updates,
            commands::get_app_snapshot,
            commands::plan_install,
            commands::plan_uninstall,
            commands::run_health_check,
            commands::mac_plan_update,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Codex App Manager");
}

