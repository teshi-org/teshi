mod commands;
mod window_state;

use std::sync::Arc;

use tauri::{Emitter, Manager};
use teshi_runtime::{HostEventCallback, RuntimeConfig, TeshiRuntime};

use crate::commands::{
    abandon_pending_locator_cmd, check_project_switch_allowed_cmd, confirm_locator_cmd,
    confirm_teardown, get_active_step_cmd, get_pending_locator_cmd, get_project_root,
    get_recent_projects_cmd, list_dir, open_project, open_project_dir, reject_locator_cmd,
    render_feature_cmd, resize_terminal, set_browser_active_cmd, set_terminal_active_cmd,
    spawn_terminal, start_browser_sidecar, stop_browser_sidecar, stop_terminal,
    sync_active_step_cmd, teardown_runtime, write_terminal,
};
use crate::window_state::{
    take_legacy_window_size_from_settings, PendingLegacyWindowSize, PERSISTED_STATE_FLAGS,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();

    tauri::Builder::default()
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(PERSISTED_STATE_FLAGS)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(path) = argv.iter().skip(1).find(|a| !a.starts_with('-')) {
                let _ = app.emit("open-project-cli", path.clone());
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .manage(PendingLegacyWindowSize(std::sync::Mutex::new(
            take_legacy_window_size_from_settings().ok().flatten(),
        )))
        .invoke_handler(tauri::generate_handler![
            open_project_dir,
            open_project,
            get_recent_projects_cmd,
            list_dir,
            get_project_root,
            render_feature_cmd,
            sync_active_step_cmd,
            get_active_step_cmd,
            get_pending_locator_cmd,
            confirm_locator_cmd,
            reject_locator_cmd,
            abandon_pending_locator_cmd,
            start_browser_sidecar,
            stop_browser_sidecar,
            spawn_terminal,
            stop_terminal,
            resize_terminal,
            write_terminal,
            check_project_switch_allowed_cmd,
            confirm_teardown,
            teardown_runtime,
            set_browser_active_cmd,
            set_terminal_active_cmd,
            finalize_main_window_cmd,
        ])
        .setup(|app| {
            let script = app
                .path()
                .resolve(
                    "resources/browser_service.py",
                    tauri::path::BaseDirectory::Resource,
                )
                .map_err(|e| e.to_string())?;

            let handle = app.handle().clone();
            let host: HostEventCallback = Arc::new(move |name: &str, payload: serde_json::Value| {
                let _ = handle.emit(name, payload);
            });

            let rt = TeshiRuntime::new(
                RuntimeConfig {
                    browser_service_script: script,
                },
                Some(host),
            );
            rt.emit_initial_recent();
            app.manage(rt);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running teshi-desktop");
}

fn init_logging() {
    let log_dir = teshi_runtime::app_data_dir()
        .map(|p| p.join("logs"))
        .unwrap_or_else(|_| std::env::temp_dir().join("teshi-desktop-logs"));
    let file_appender = tracing_appender::rolling::daily(log_dir, "teshi-desktop.log");
    let (writer, guard) = tracing_appender::non_blocking(file_appender);
    Box::leak(Box::new(guard));
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_writer(writer)
        .with_ansi(false)
        .try_init();
}

/// Post-restore hook: legacy migration, work-area clamp, and center for windowed mode.
#[tauri::command]
fn finalize_main_window_cmd(
    window: tauri::WebviewWindow,
    pending: tauri::State<'_, PendingLegacyWindowSize>,
) -> Result<(), String> {
    crate::window_state::finalize_main_window(&window, &pending).map_err(|e| e.to_string())
}
