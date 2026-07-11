pub mod adapters;
pub mod app;
pub mod commands;
pub mod domain;
pub mod errors;

mod state;

use std::sync::atomic::Ordering;

use tauri::{Emitter, Manager, RunEvent, WindowEvent};

use crate::app::op_phase::QuitPolicy;

/// The "ask before closing" setting, read fresh from disk so a toggle in
/// Settings takes effect immediately (no restart).
fn confirm_close_enabled() -> bool {
    crate::app::settings_store::AppSettings::load().confirm_close
}

/// Unified quit/close policy: phase-aware + confirm_close setting.
fn quit_policy_for(app: &tauri::AppHandle) -> QuitPolicy {
    let state = app.state::<state::ManagerState>();
    let force = state.force_quit.load(Ordering::SeqCst);
    state
        .operations
        .quit_policy(force, confirm_close_enabled())
}

/// Apply a quit policy decision for window/menu/exit paths.
/// Returns `true` when the caller should proceed to exit.
fn apply_quit_policy(app: &tauri::AppHandle, policy: &QuitPolicy) -> bool {
    match policy {
        QuitPolicy::Allow => true,
        QuitPolicy::Confirm => {
            let _ = app.emit("app://confirm-quit", ());
            false
        }
        QuitPolicy::Block {
            phase,
            reason_code,
            reason,
            kind,
        } => {
            log::warn!(
                "quit blocked phase={} reason_code={reason_code} kind={:?} reason={reason}",
                phase.as_str(),
                kind
            );
            let _ = app.emit("app://quit-blocked", policy);
            false
        }
    }
}

/// The bundled custom-protocol scheme differs by platform. Windows uses the
/// HTTP(S) compatibility origin selected by `useHttpsScheme`; desktop WebKit
/// uses the `tauri:` origin. Keep this decision next to the navigation gate so
/// a future config change cannot silently widen the allowlist.
fn bundled_app_scheme(use_https_scheme: bool) -> &'static str {
    #[cfg(target_os = "windows")]
    {
        if use_https_scheme {
            "https"
        } else {
            "http"
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = use_https_scheme;
        "tauri"
    }
}

/// Only the exact bundled app origin may replace the top-level document.
/// External links already go through the validated `open_url` command and
/// system shell.
fn is_allowed_app_navigation(url: &url::Url, allow_dev_server: bool, bundled_scheme: &str) -> bool {
    let expected_host = if bundled_scheme == "tauri" {
        "localhost"
    } else {
        "tauri.localhost"
    };
    let expected_port = match bundled_scheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    let bundled_origin = url.scheme() == bundled_scheme
        && url.host_str() == Some(expected_host)
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == expected_port;
    if bundled_origin {
        return true;
    }

    allow_dev_server
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1") | Some("localhost"))
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(1420)
}

#[cfg(any(target_os = "windows", test))]
fn browser_accelerators_enabled(is_dev: bool) -> bool {
    is_dev
}

fn initial_main_window_visibility(
    configured_visible: bool,
    is_windows: bool,
    is_dev: bool,
) -> bool {
    configured_visible && (!is_windows || is_dev)
}

#[cfg(target_os = "windows")]
fn show_windows_startup_error(detail: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let title: Vec<u16> = "Codex App Manager startup error\0".encode_utf16().collect();
    let body: Vec<u16> = format!(
        "The secure Windows interface could not be initialized. The app will close.\n\n{detail}\0"
    )
    .encode_utf16()
    .collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(target_os = "windows")]
fn abort_for_unsafe_windows_webview(app: &tauri::AppHandle, detail: &str) {
    log::error!("WebView2 release safety gate failed: {detail}");
    let state = app.state::<state::ManagerState>();
    state.webview_gate_failed.store(true, Ordering::SeqCst);
    show_windows_startup_error(detail);
    state.force_quit.store(true, Ordering::SeqCst);
    app.exit(1);
}

/// WebView2 handles browser accelerators before DOM `keydown`, so renderer
/// `preventDefault()` cannot stop Ctrl+P, Ctrl+R or F5. Disable that native
/// layer in release while retaining Tauri's built-in shortcuts for `tauri dev`.
#[cfg(target_os = "windows")]
fn configure_windows_browser_accelerators(
    app: &tauri::App,
    window: &tauri::WebviewWindow<tauri::Wry>,
    enabled: bool,
    show_after_gate: bool,
) -> tauri::Result<()> {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
    use windows_core::Interface;

    if enabled {
        return Ok(());
    }

    let label = window.label().to_string();
    let callback_label = label.clone();
    let app_handle = app.handle().clone();
    let gated_window = window.clone();
    let callback_result = window.with_webview(move |platform_webview| {
        let result = unsafe {
            (|| -> windows_core::Result<()> {
                let webview = platform_webview.controller().CoreWebView2()?;
                let settings = webview.Settings()?;
                let settings3 = settings.cast::<ICoreWebView2Settings3>()?;
                settings3.SetAreBrowserAcceleratorKeysEnabled(false)
            })()
        };
        match result {
            Ok(()) => {
                if !show_after_gate {
                    app_handle
                        .state::<state::ManagerState>()
                        .webview_safe_to_show
                        .store(true, Ordering::SeqCst);
                    log::info!(
                        "disabled native WebView2 browser accelerators window={callback_label}"
                    );
                } else {
                    match gated_window.show() {
                        Ok(()) => {
                            app_handle
                                .state::<state::ManagerState>()
                                .webview_safe_to_show
                                .store(true, Ordering::SeqCst);
                            log::info!(
                                "disabled native WebView2 browser accelerators and opened window={callback_label}"
                            );
                        }
                        Err(error) => abort_for_unsafe_windows_webview(
                            &app_handle,
                            &format!("failed to show gated window={callback_label}: {error}"),
                        ),
                    }
                }
            }
            Err(error) => abort_for_unsafe_windows_webview(
                &app_handle,
                &format!(
                    "failed to disable browser accelerators window={callback_label}: {error}"
                ),
            ),
        }
    });
    if let Err(error) = callback_result {
        let detail = format!("failed to schedule WebView2 safety gate window={label}: {error}");
        log::error!("{detail}");
        show_windows_startup_error(&detail);
        return Err(error);
    }
    Ok(())
}

/// Build the configured main window ourselves so the native webview receives
/// navigation and new-window handlers before its first document is loaded.
fn build_main_window(app: &tauri::App) -> tauri::Result<()> {
    let mut config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .ok_or_else(|| {
            tauri::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "missing main window config",
            ))
        })?
        .clone();

    #[cfg(target_os = "windows")]
    let configured_visible = config.visible;
    config.visible =
        initial_main_window_visibility(config.visible, cfg!(target_os = "windows"), cfg!(dev));

    let bundled_scheme = bundled_app_scheme(config.use_https_scheme);
    let window = tauri::WebviewWindowBuilder::from_config(app, &config)?
        .on_navigation(move |url| {
            // `cfg(dev)` is emitted by Tauri itself and remains true for the
            // supported `tauri dev --release` mode. `debug_assertions` does not.
            let allowed = is_allowed_app_navigation(url, cfg!(dev), bundled_scheme);
            if !allowed {
                log::warn!(
                    "blocked top-level webview navigation scheme={} host={}",
                    url.scheme(),
                    url.host_str().unwrap_or("<none>")
                );
            }
            allowed
        })
        .on_new_window(|url, _features| {
            log::warn!(
                "blocked webview new-window request scheme={} host={}",
                url.scheme(),
                url.host_str().unwrap_or("<none>")
            );
            tauri::webview::NewWindowResponse::Deny
        })
        .build()?;
    #[cfg(target_os = "windows")]
    configure_windows_browser_accelerators(
        app,
        &window,
        browser_accelerators_enabled(cfg!(dev)),
        configured_visible,
    )?;
    #[cfg(not(target_os = "windows"))]
    let _ = window;
    Ok(())
}

/// macOS routes Cmd+Q through the app-menu Quit item, which terminates *below*
/// the RunEvent loop (so ExitRequested can't hold it). Replace the default menu
/// with one whose Quit item is ours — its activation lands in `on_menu_event`
/// where we can confirm first. The Edit submenu is preserved so the standard
/// copy/paste/select-all shortcuts keep working in text fields.
#[cfg(target_os = "macos")]
fn install_macos_menu(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{AboutMetadata, MenuBuilder, MenuItemBuilder, SubmenuBuilder};

    let quit = MenuItemBuilder::with_id("cam-quit", "Quit Codex App 管理器")
        .accelerator("Cmd+Q")
        .build(app)?;
    let app_menu = SubmenuBuilder::new(app, "Codex App 管理器")
        .about(Some(AboutMetadata::default()))
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .item(&quit)
        .build()?;
    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let window_menu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .close_window()
        .build()?;
    let menu = MenuBuilder::new(app)
        .items(&[&app_menu, &edit_menu, &window_menu])
        .build()?;
    app.set_menu(menu)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            #[cfg(target_os = "windows")]
            if !app
                .state::<state::ManagerState>()
                .webview_safe_to_show
                .load(Ordering::SeqCst)
            {
                log::warn!("ignored second-instance focus before WebView2 safety gate completed");
                return;
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("codex-app-manager".to_string()),
                    }),
                ])
                .level(if cfg!(debug_assertions) {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Info
                })
                .level_for("tao", log::LevelFilter::Warn)
                .level_for("wry", log::LevelFilter::Warn)
                .max_file_size(crate::app::logging::MAX_LOG_FILE_BYTES)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepAll)
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .format(|out, message, record| {
                    out.finish(format_args!(
                        "[{}] [{}] [{}:{}] {}",
                        record.level(),
                        record.target(),
                        record.file().unwrap_or("?"),
                        record.line().unwrap_or(0),
                        message
                    ))
                })
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        // Launch-at-login support. Off by default — the user opts in from
        // Settings; we only register the plugin so the toggle can flip it.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(state::ManagerState::new())
        .invoke_handler(tauri::generate_handler![
            commands::mac_plan_update,
            commands::mac_stage_update,
            commands::mac_perform_update,
            commands::mac_status,
            commands::mac_adopt,
            commands::mac_pick_existing_install,
            commands::mac_adopt_path,
            commands::mac_install,
            commands::mac_pause_download,
            commands::mac_cancel_download,
            commands::mac_discard_download,
            commands::mac_launch_codex,
            commands::mac_uninstall,
            commands::manager_check_update,
            commands::manager_install_update,
            commands::get_settings,
            commands::set_settings,
            commands::get_config_health,
            commands::restore_config_backup,
            commands::reset_config,
            commands::retry_ancillary,
            commands::begin_operation,
            commands::arm_destructive,
            commands::end_operation,
            commands::get_operation_snapshot,
            commands::get_operation_completion,
            commands::confirm_quit,
            commands::win_default_install_root,
            commands::win_pick_install_dir,
            commands::win_set_install_root,
            commands::win_reset_install_root,
            commands::get_autostart,
            commands::set_autostart,
            commands::open_url,
            commands::win_plan_update,
            commands::win_stage_update,
            commands::win_auto_stage_update,
            commands::win_pause_download,
            commands::win_cancel_download,
            commands::win_discard_download,
            commands::win_status,
            commands::win_adopt,
            commands::win_pick_existing_install,
            commands::win_adopt_path,
            commands::win_launch_codex,
            commands::win_perform_update,
            commands::win_uninstall,
            commands::get_diagnostics,
            commands::open_logs_dir,
            commands::open_codex_home,
            commands::log_frontend_error,
        ])
        .setup(|app| {
            build_main_window(app)?;
            #[cfg(target_os = "macos")]
            install_macos_menu(app)?;
            log::info!(
                "Codex App Manager v{} starting (os={}, arch={})",
                app.package_info().version,
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            if let Some(logs_dir) = crate::app::logging::logs_dir(app.handle()) {
                tauri::async_runtime::spawn_blocking(move || {
                    crate::app::logging::prune_old_logs(
                        &logs_dir,
                        crate::app::logging::KEEP_LOG_FILES,
                    );
                });
            }
            let operations = app.state::<state::ManagerState>().operations.clone();
            #[cfg(target_os = "windows")]
            let recovery_app = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                #[cfg(target_os = "windows")]
                loop {
                    match recovery_app
                        .state::<state::ManagerState>()
                        .webview_startup_gate()
                    {
                        state::WebviewStartupGate::Proceed => break,
                        state::WebviewStartupGate::Abort => {
                            log::warn!(
                                "startup recovery skipped after WebView2 safety gate failure"
                            );
                            return;
                        }
                        state::WebviewStartupGate::Wait => {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                }
                // Crash-safe install recovery MUST run before ordinary staging
                // cleanup so recovery materials (backup / staged new) are not
                // deleted out from under an incomplete swap.
                let recovery =
                    crate::app::install_tx::recover_pending_transactions(Some(&operations));
                if recovery.failed > 0 || recovery.kept_manual > 0 {
                    log::warn!(
                        "install transaction recovery finished scanned={} continued={} rolled_back={} completed={} cleared={} kept_manual={} failed={}",
                        recovery.scanned,
                        recovery.continued,
                        recovery.rolled_back,
                        recovery.completed,
                        recovery.cleared,
                        recovery.kept_manual,
                        recovery.failed
                    );
                }
                let summary = crate::app::staging::cleanup_stale_staging(&operations);
                if summary.failed > 0 {
                    log::warn!(
                        "staging cleanup completed with failures scanned={} removed={} failed={}",
                        summary.scanned,
                        summary.removed,
                        summary.failed
                    );
                }
            });
            let health = app
                .state::<state::ManagerState>()
                .config_health
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .clone();
            if !health.is_ok() {
                log::warn!(
                    "config health not ok: settings={} provenance={} unknown_source={:?} detail={:?}",
                    health.settings_status,
                    health.provenance_status,
                    health.unknown_source,
                    health.detail
                );
                let _ = app.emit("app://config-health", health);
            }
            Ok(())
        })
        // Our custom macOS Quit item lands here (Cmd+Q). Same phase-aware policy
        // as window close / ExitRequested.
        .on_menu_event(|app, event| {
            if event.id().0.as_str() == "cam-quit" {
                let policy = quit_policy_for(app);
                if apply_quit_policy(app, &policy) {
                    app.exit(0);
                }
            }
        })
        // A normal "open it when you need it" app — NOT a menu-bar resident.
        // Closing the window quits the process so nothing lingers in the
        // background; the Dock icon is the only entry point, and login launch is
        // an explicit, off-by-default opt-in (see Settings).
        //
        // The window has no system chrome, so every window-close path — the
        // in-app ✕, Alt+F4, the macOS window close — arrives here. Policy is
        // phase-aware: point-of-no-return install steps block quit; otherwise
        // the confirm_close setting may raise a dialog.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let policy = quit_policy_for(app);
                if apply_quit_policy(app, &policy) {
                    app.exit(0);
                } else {
                    api.prevent_close();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build Codex App Manager")
        // Cmd+Q (and any other app-level quit) lands as ExitRequested rather than
        // a window CloseRequested — gate it with the same phase-aware policy.
        .run(|app, event| {
            if let RunEvent::ExitRequested { api, .. } = event {
                let policy = quit_policy_for(app);
                if !apply_quit_policy(app, &policy) {
                    api.prevent_exit();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{
        browser_accelerators_enabled, initial_main_window_visibility, is_allowed_app_navigation,
    };

    #[test]
    fn native_browser_accelerators_are_release_only_disabled() {
        assert!(!browser_accelerators_enabled(false));
        assert!(browser_accelerators_enabled(true));
    }

    #[test]
    fn windows_release_window_stays_hidden_until_the_native_gate_succeeds() {
        assert!(!initial_main_window_visibility(true, true, false));
        assert!(initial_main_window_visibility(true, true, true));
        assert!(initial_main_window_visibility(true, false, false));
        assert!(!initial_main_window_visibility(false, true, true));
    }

    #[test]
    fn navigation_policy_allows_only_the_selected_bundled_origin_in_release() {
        for allowed in ["tauri://localhost/", "tauri://localhost/assets/app.js"] {
            let url = url::Url::parse(allowed).unwrap();
            assert!(is_allowed_app_navigation(&url, false, "tauri"), "{allowed}");
        }

        for blocked in [
            "http://tauri.localhost/",
            "https://tauri.localhost/",
            "tauri://localhost:1420/",
            "tauri://user@localhost/",
            "https://github.com/Wangnov/Codex-App-Manager",
            "javascript:alert(1)",
            "data:text/html,boom",
            "file:///tmp/unsafe.html",
            "http://127.0.0.1:1420/",
        ] {
            let url = url::Url::parse(blocked).unwrap();
            assert!(
                !is_allowed_app_navigation(&url, false, "tauri"),
                "{blocked}"
            );
        }
    }

    #[test]
    fn navigation_policy_honors_the_configured_windows_scheme_and_port() {
        let http = url::Url::parse("http://tauri.localhost/assets/app.js").unwrap();
        let https = url::Url::parse("https://tauri.localhost/assets/app.js").unwrap();
        let alternate_port = url::Url::parse("http://tauri.localhost:1420/").unwrap();

        assert!(is_allowed_app_navigation(&http, false, "http"));
        assert!(!is_allowed_app_navigation(&https, false, "http"));
        assert!(!is_allowed_app_navigation(&alternate_port, false, "http"));
        assert!(is_allowed_app_navigation(&https, false, "https"));
        assert!(!is_allowed_app_navigation(&http, false, "https"));
    }

    #[test]
    fn navigation_policy_limits_development_to_the_configured_loopback_port() {
        assert!(is_allowed_app_navigation(
            &url::Url::parse("http://127.0.0.1:1420/").unwrap(),
            true,
            "tauri"
        ));
        assert!(is_allowed_app_navigation(
            &url::Url::parse("http://localhost:1420/src/main.tsx").unwrap(),
            true,
            "tauri"
        ));
        assert!(!is_allowed_app_navigation(
            &url::Url::parse("http://127.0.0.1:3000/").unwrap(),
            true,
            "tauri"
        ));
        assert!(!is_allowed_app_navigation(
            &url::Url::parse("https://example.com:1420/").unwrap(),
            true,
            "tauri"
        ));
    }
}
