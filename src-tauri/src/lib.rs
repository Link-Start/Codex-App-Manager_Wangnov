pub mod adapters;
pub mod app;
pub mod commands;
pub mod domain;
pub mod errors;

mod state;

use std::sync::atomic::Ordering;

use tauri::webview::PageLoadEvent;
use tauri::{Emitter, Manager, RunEvent, UserAttentionType, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::app::op_phase::QuitPolicy;
use crate::app::shell::{
    FrontendReadyResult, FrontendToken, NativeLocale, ShellDispatch, ShellEvent, PRODUCT_NAME,
};

const FRONTEND_READY_GLOBAL: &str = "__CAM_FRONTEND_READY__";
const FRONTEND_READY_EVENT: &str = "cam:frontend-readiness";

/// The "ask before closing" setting, read fresh from disk so a toggle in
/// Settings takes effect immediately (no restart).
fn confirm_close_enabled() -> bool {
    crate::app::settings_store::AppSettings::load().confirm_close
}

/// Unified quit/close policy: phase-aware + confirm_close setting.
fn quit_policy_for(app: &tauri::AppHandle) -> QuitPolicy {
    let state = app.state::<state::ManagerState>();
    let force = state.force_quit.load(Ordering::SeqCst);
    state.operations.quit_policy(force, confirm_close_enabled())
}

/// Apply a quit policy decision for window/menu/exit paths.
/// Returns `true` when the caller should proceed to exit.
fn apply_quit_policy(app: &tauri::AppHandle, policy: &QuitPolicy) -> bool {
    match policy {
        QuitPolicy::Allow => true,
        QuitPolicy::Confirm => {
            dispatch_shell_event(app, ShellEvent::ConfirmQuit);
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
            dispatch_shell_event(app, ShellEvent::QuitBlocked(policy.clone()));
            false
        }
    }
}

fn request_main_window_attention(app: &tauri::AppHandle, reason: &str) {
    let Some(window) = app.get_webview_window("main") else {
        log::error!("main window unavailable reason={reason}");
        return;
    };
    match window.request_user_attention(Some(UserAttentionType::Informational)) {
        Ok(()) => log::info!("main window requested user attention reason={reason}"),
        Err(error) => {
            log::warn!("main window request-attention failed reason={reason} error={error}")
        }
    }
}

fn restore_main_window(app: &tauri::AppHandle, reason: &str) {
    let Some(window) = app.get_webview_window("main") else {
        log::error!("main window restore failed reason={reason} error=window-missing");
        return;
    };

    let mut failed = false;
    for (step, result) in [
        ("unminimize", window.unminimize()),
        ("show", window.show()),
        ("focus", window.set_focus()),
    ] {
        if let Err(error) = result {
            failed = true;
            log::warn!("main window restore step failed reason={reason} step={step} error={error}");
        }
    }

    let focused = match window.is_focused() {
        Ok(focused) => focused,
        Err(error) => {
            failed = true;
            log::warn!("main window focus probe failed reason={reason} error={error}");
            false
        }
    };
    let degraded = failed || !focused;
    if degraded {
        request_main_window_attention(app, reason);
    }
    log::info!("main window restored reason={reason} focused={focused} degraded={degraded}");
}

fn emit_shell_event(app: &tauri::AppHandle, event: &ShellEvent) {
    let result = match event {
        ShellEvent::ConfirmQuit => app.emit("app://confirm-quit", ()),
        ShellEvent::QuitBlocked(policy) => app.emit("app://quit-blocked", policy),
    };
    match result {
        Ok(()) => log::info!("shell event emitted kind={}", event.kind()),
        Err(error) => {
            log::warn!(
                "shell event emit failed kind={} error={error}",
                event.kind()
            );
            request_main_window_attention(app, "shell-event-emit-failed");
        }
    }
}

pub(crate) fn exit_after_confirm(app: &tauri::AppHandle) {
    let _ = codex_mac_engine::cancel_active_download();
    let _ = codex_win_engine::cancel_active_download();
    app.state::<state::ManagerState>()
        .force_quit
        .store(true, Ordering::SeqCst);
    app.exit(0);
}

fn native_confirm_allows_exit(policy: &QuitPolicy) -> bool {
    !matches!(policy, QuitPolicy::Block { .. })
}

fn finish_native_shell_dialog(app: tauri::AppHandle) {
    let next = app
        .state::<state::ManagerState>()
        .frontend
        .native_dialog_finished();
    if let Some(event) = next {
        show_native_shell_event(app, event);
    }
}

fn show_native_shell_event(app: tauri::AppHandle, event: ShellEvent) {
    let kind = event.kind();
    log::warn!("shell event using native fallback kind={kind}");
    restore_main_window(&app, "native-shell-fallback");

    match event {
        ShellEvent::ConfirmQuit => {
            let mut dialog = app
                .dialog()
                .message(
                    "The interface is not responding. Quit Codex App Manager safely?\n\n\
                     界面没有响应。是否安全退出 Codex App Manager？",
                )
                .title(PRODUCT_NAME)
                .kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCancelCustom(
                    "Quit / 退出".to_string(),
                    "Keep Open / 保持打开".to_string(),
                ));
            if let Some(window) = app.get_webview_window("main") {
                dialog = dialog.parent(&window);
            }
            dialog.show(move |confirmed| {
                if confirmed {
                    let policy = quit_policy_for(&app);
                    if !native_confirm_allows_exit(&policy) {
                        log::warn!("native quit confirmation recheck blocked policy={policy:?}");
                        dispatch_shell_event(&app, ShellEvent::QuitBlocked(policy));
                    } else {
                        log::info!("native quit confirmation accepted");
                        exit_after_confirm(&app);
                        return;
                    }
                } else {
                    log::info!("native quit confirmation cancelled");
                }
                finish_native_shell_dialog(app);
            });
        }
        ShellEvent::QuitBlocked(policy) => {
            let reason = match &policy {
                QuitPolicy::Block { reason, .. } => reason.as_str(),
                _ => "A protected operation is still active.",
            };
            let mut dialog = app
                .dialog()
                .message(format!(
                    "Codex App Manager must stay open until the protected step finishes.\n\
                     {reason}\n\n受保护步骤完成前，Codex App Manager 必须保持打开。"
                ))
                .title(PRODUCT_NAME)
                .kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCustom("OK / 知道了".to_string()));
            if let Some(window) = app.get_webview_window("main") {
                dialog = dialog.parent(&window);
            }
            dialog.show(move |_| finish_native_shell_dialog(app));
        }
    }
}

pub(crate) fn dispatch_shell_event(app: &tauri::AppHandle, event: ShellEvent) {
    let kind = event.kind();
    match app.state::<state::ManagerState>().frontend.route(event) {
        ShellDispatch::Emit(event) => emit_shell_event(app, &event),
        ShellDispatch::Native(event) => show_native_shell_event(app.clone(), event),
        ShellDispatch::Queued { pending } => {
            log::info!("shell event queued kind={kind} pending={pending} frontend_ready=false");
            request_main_window_attention(app, "shell-event-queued");
        }
    }
}

/// macOS routes Cmd+Q through the app-menu Quit item, which terminates *below*
/// the RunEvent loop (so ExitRequested can't hold it). Replace the default menu
/// with one whose Quit item is ours — its activation lands in `on_menu_event`
/// where we can confirm first. The Edit submenu is preserved so the standard
/// copy/paste/select-all shortcuts keep working in text fields.
#[cfg(target_os = "macos")]
fn install_macos_menu(app: &tauri::AppHandle, locale: NativeLocale) -> tauri::Result<()> {
    use tauri::menu::{AboutMetadata, MenuBuilder, MenuItemBuilder, SubmenuBuilder};

    let copy = locale.menu();
    let metadata = AboutMetadata {
        name: Some(PRODUCT_NAME.to_string()),
        version: Some(app.package_info().version.to_string()),
        short_version: Some(app.package_info().version.to_string()),
        copyright: app.config().bundle.copyright.clone(),
        ..AboutMetadata::default()
    };
    let quit = MenuItemBuilder::with_id("cam-quit", copy.quit)
        .accelerator("Cmd+Q")
        .build(app)?;
    let minimize = MenuItemBuilder::with_id("cam-minimize", copy.minimize)
        .accelerator("Cmd+M")
        .build(app)?;
    let close = MenuItemBuilder::with_id("cam-close", copy.close_window)
        .accelerator("Cmd+W")
        .build(app)?;
    let app_menu = SubmenuBuilder::new(app, PRODUCT_NAME)
        .about_with_text(copy.about, Some(metadata))
        .separator()
        .services_with_text(copy.services)
        .separator()
        .hide_with_text(copy.hide)
        .hide_others_with_text(copy.hide_others)
        .show_all_with_text(copy.show_all)
        .separator()
        .item(&quit)
        .build()?;
    let edit_menu = SubmenuBuilder::new(app, copy.edit)
        .undo_with_text(copy.undo)
        .redo_with_text(copy.redo)
        .separator()
        .cut_with_text(copy.cut)
        .copy_with_text(copy.copy)
        .paste_with_text(copy.paste)
        .select_all_with_text(copy.select_all)
        .build()?;
    let window_menu = SubmenuBuilder::new(app, copy.window)
        .items(&[&minimize, &close])
        .build()?;
    let menu = MenuBuilder::new(app)
        .items(&[&app_menu, &edit_menu, &window_menu])
        .build()?;
    app.set_menu(menu)?;
    log::info!(
        "native menu installed lang={} product={} about_version={}",
        locale.code(),
        PRODUCT_NAME,
        app.package_info().version
    );
    Ok(())
}

/// The frontend calls this only after both quit listeners are registered.
/// The first handshake reveals the initially hidden window and drains any
/// close/quit decisions queued during WebView startup. Later calls only update
/// the native menu when the user changes the application language.
#[tauri::command]
fn frontend_ready(
    app: tauri::AppHandle,
    state: tauri::State<'_, state::ManagerState>,
    lang: String,
    generation: u64,
    token: String,
) -> Result<(), String> {
    let ready = match state.frontend.mark_ready(generation, &token) {
        FrontendReadyResult::Accepted(ready) => ready,
        FrontendReadyResult::Stale { current_generation } => {
            log::warn!(
                "stale frontend ready rejected lang={} generation={generation} current_generation={current_generation}",
                NativeLocale::from_tag(&lang).code()
            );
            return Err("stale frontend readiness token".to_string());
        }
    };
    let locale = NativeLocale::from_tag(&lang);
    #[cfg(target_os = "macos")]
    if let Err(error) = install_macos_menu(&app, locale) {
        log::warn!(
            "native menu update failed lang={} error={error}",
            locale.code()
        );
    }

    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window.set_title(PRODUCT_NAME) {
            log::warn!("main window title update failed error={error}");
        }
    } else {
        log::error!("frontend ready but main window is unavailable");
    }

    log::info!(
        "frontend ready lang={} generation={} first_ready={} degraded={} activation_pending={} pending_events={}",
        locale.code(),
        ready.generation,
        ready.first_ready,
        ready.degraded,
        ready.activation_pending,
        ready.pending.len()
    );
    if ready.first_ready {
        let reason = if ready.activation_pending {
            "frontend-ready-single-instance"
        } else {
            "frontend-ready"
        };
        restore_main_window(&app, reason);
    }
    for event in ready.pending {
        emit_shell_event(&app, &event);
    }
    Ok(())
}

fn frontend_token_script(readiness: &FrontendToken) -> String {
    let encoded = serde_json::to_string(&serde_json::json!({
        "generation": readiness.generation,
        "token": readiness.token,
    }))
    .expect("frontend readiness is JSON serializable");
    format!(
        "(() => {{ const readiness = Object.freeze({encoded}); \
         Object.defineProperty(window, '{FRONTEND_READY_GLOBAL}', \
         {{ value: readiness, configurable: true }}); \
         window.dispatchEvent(new CustomEvent('{FRONTEND_READY_EVENT}', \
         {{ detail: readiness }})); }})();"
    )
}

fn schedule_frontend_ready_fallback(app: tauri::AppHandle, generation: u64) {
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(std::time::Duration::from_secs(10));
        let Some(degraded) = app
            .state::<state::ManagerState>()
            .frontend
            .mark_degraded(generation)
        else {
            return;
        };
        log::error!(
            "frontend readiness timed out after 10s; entering native degraded mode generation={} activation_pending={} native_event_pending={}",
            generation,
            degraded.activation_pending,
            degraded.next_native_event.is_some()
        );
        restore_main_window(&app, "frontend-ready-timeout");
        if degraded.activation_pending {
            restore_main_window(&app, "frontend-ready-timeout-single-instance");
        }
        if let Some(event) = degraded.next_native_event {
            show_native_shell_event(app, event);
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            log::info!(
                "single-instance activation requested args_count={} cwd_present={}",
                args.len(),
                !cwd.is_empty()
            );
            if app
                .state::<state::ManagerState>()
                .frontend
                .request_activation()
            {
                restore_main_window(app, "single-instance");
            } else {
                log::info!("single-instance activation queued frontend_ready=false");
                request_main_window_attention(app, "single-instance-queued");
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
            frontend_ready,
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
            #[cfg(target_os = "macos")]
            install_macos_menu(app.handle(), NativeLocale::En)?;
            log::info!(
                "Codex App Manager v{} starting (os={}, arch={})",
                app.package_info().version,
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            if let Some(run_id) = crate::app::paths::packaged_smoke_run_id() {
                log::info!("packaged smoke run id={run_id} data_dir_isolated=true");
            }
            if let Some(logs_dir) = crate::app::logging::logs_dir(app.handle()) {
                tauri::async_runtime::spawn_blocking(move || {
                    crate::app::logging::prune_old_logs(
                        &logs_dir,
                        crate::app::logging::KEEP_LOG_FILES,
                    );
                });
            }
            let operations = app.state::<state::ManagerState>().operations.clone();
            tauri::async_runtime::spawn_blocking(move || {
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
        .on_page_load(|webview, payload| {
            if webview.label() != "main" {
                return;
            }
            let app = webview.app_handle();
            match payload.event() {
                PageLoadEvent::Started => {
                    let load = app
                        .state::<state::ManagerState>()
                        .frontend
                        .mark_loading();
                    log::info!("frontend page loading generation={}", load.generation);
                    schedule_frontend_ready_fallback(app.clone(), load.generation);
                }
                PageLoadEvent::Finished => {
                    let token = app
                        .state::<state::ManagerState>()
                        .frontend
                        .current_token();
                    let Some(token) = token else {
                        log::error!("frontend page finished without a readiness token");
                        return;
                    };
                    match webview.eval(frontend_token_script(&token)) {
                        Ok(()) => log::info!(
                            "frontend readiness token injected generation={}",
                            token.generation
                        ),
                        Err(error) => log::error!(
                            "frontend readiness token injection failed generation={} error={error}",
                            token.generation
                        ),
                    }
                }
            }
        })
        // Our custom macOS Quit item lands here (Cmd+Q). Same phase-aware policy
        // as window close / ExitRequested.
        .on_menu_event(|app, event| {
            match event.id().0.as_str() {
                "cam-quit" => {
                    log::info!("menu quit requested id=cam-quit");
                    let policy = quit_policy_for(app);
                    if apply_quit_policy(app, &policy) {
                        app.exit(0);
                    }
                }
                "cam-minimize" => {
                    log::info!("menu minimize requested id=cam-minimize");
                    if let Some(window) = app.get_webview_window("main") {
                        if let Err(error) = window.minimize() {
                            log::warn!("menu minimize failed error={error}");
                            request_main_window_attention(app, "menu-minimize-failed");
                        }
                    } else {
                        log::error!("menu minimize failed error=window-missing");
                    }
                }
                "cam-close" => {
                    log::info!("menu close requested id=cam-close");
                    if let Some(window) = app.get_webview_window("main") {
                        if let Err(error) = window.close() {
                            log::warn!("menu close failed error={error}");
                            request_main_window_attention(app, "menu-close-failed");
                        }
                    } else {
                        log::error!("menu close failed error=window-missing");
                    }
                }
                _ => {}
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
                log::info!("window close requested label={}", window.label());
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
                log::info!("application exit requested");
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
        frontend_token_script, native_confirm_allows_exit, FRONTEND_READY_EVENT,
        FRONTEND_READY_GLOBAL,
    };
    use crate::app::op_phase::{OperationPhase, QuitPolicy};
    use crate::app::shell::FrontendToken;

    #[test]
    fn readiness_token_script_uses_json_encoding_and_a_document_event() {
        let script = frontend_token_script(&FrontendToken {
            generation: 7,
            token: "token-'\\-value".to_string(),
        });
        assert!(script.contains(FRONTEND_READY_GLOBAL));
        assert!(script.contains(FRONTEND_READY_EVENT));
        assert!(script.contains(r#""generation":7"#));
        assert!(script.contains(r#"token-'\\-value"#));
        assert!(script.contains("configurable: true"));
        assert!(script.contains("Object.freeze"));
    }

    #[test]
    fn native_confirmation_never_exits_a_protected_phase() {
        assert!(native_confirm_allows_exit(&QuitPolicy::Allow));
        assert!(native_confirm_allows_exit(&QuitPolicy::Confirm));
        assert!(!native_confirm_allows_exit(&QuitPolicy::Block {
            phase: OperationPhase::Committing,
            reason_code: "committing".to_string(),
            reason: "protected".to_string(),
            kind: Some("update".to_string()),
        }));
    }
}
