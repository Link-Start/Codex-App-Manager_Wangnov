pub mod adapters;
pub mod app;
pub mod commands;
pub mod domain;
pub mod errors;
pub mod ports;

mod state;

use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};
use tauri_plugin_positioner::{Position, WindowExt};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(state::ManagerState::new())
        .invoke_handler(tauri::generate_handler![
            commands::check_payload_updates,
            commands::get_app_snapshot,
            commands::plan_install,
            commands::plan_uninstall,
            commands::run_health_check,
            commands::mac_plan_update,
            commands::mac_stage_update,
            commands::mac_perform_update,
            commands::mac_status,
            commands::mac_adopt,
            commands::mac_install,
            commands::mac_uninstall,
            commands::get_settings,
            commands::set_settings,
        ])
        // Menu-bar popover behaviour: dismiss when focus leaves the window.
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(false) = event {
                let _ = window.hide();
            }
        })
        .setup(|app| {
            // A true menu-bar app: no Dock icon, lives in the status bar.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let quit = MenuItemBuilder::with_id("quit", "退出 Codex App 管理器").build(app)?;
            let menu = MenuBuilder::new(app).item(&quit).build()?;

            let mut tray = TrayIconBuilder::with_id("codex-app-manager-tray")
                .tooltip("Codex App 管理器")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id() == "quit" {
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    // Keep the positioner aware of the tray geometry.
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(win) = tray.app_handle().get_webview_window("main") {
                            if win.is_visible().unwrap_or(false) {
                                let _ = win.hide();
                            } else {
                                let _ = win.move_window(Position::TrayBottomCenter);
                                let _ = win.show();
                                let _ = win.set_focus();
                            }
                        }
                    }
                });
            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            tray.build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Codex App Manager");
}
