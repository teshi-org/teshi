mod app_data;
mod gherkin_cmd;
mod locator;
mod menu;
mod project;
mod sidecar;
mod terminal;
mod watcher;
mod window_state;

use tauri::{Emitter, Manager};

use crate::app_data::get_recent_projects;
use crate::gherkin_cmd::render_feature_cmd;
use crate::locator::{
    abandon_pending_locator_cmd, confirm_locator_cmd, get_active_step_cmd, get_pending_locator_cmd,
    reject_locator_cmd, sync_active_step_cmd, LocatorWatcherState,
};
use crate::project::{
    check_project_switch_allowed, get_project_root, list_dir, open_project, set_browser_active,
    set_terminal_active, teardown_runtime, ProjectState,
};
use crate::sidecar::{
    confirm_teardown, get_recent_projects_cmd, open_project_dir, start_browser_sidecar,
    stop_browser_sidecar, SidecarState,
};
use crate::terminal::{
    resize_terminal, spawn_terminal, stop_terminal, write_terminal, TerminalState,
};
use crate::watcher::FileWatcherState;
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
        .manage(ProjectState::new())
        .manage(SidecarState::new())
        .manage(TerminalState::new())
        .manage(FileWatcherState::new())
        .manage(LocatorWatcherState::new())
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
            check_project_switch_allowed,
            confirm_teardown,
            teardown_runtime,
            set_browser_active,
            set_terminal_active,
            finalize_main_window_cmd,
        ])
        .setup(|app| {
            if let Ok(recent) = get_recent_projects() {
                let _ = app.emit("recent-loaded", recent);
            }
            let handle = app.handle().clone();
            if let Ok(menu) = menu::build_app_menu(
                &handle,
                &get_recent_projects().unwrap_or_default(),
            ) {
                let _ = app.set_menu(menu);
            }
            app.on_menu_event(move |app_handle, event| {
                menu::handle_menu_event(app_handle, event.id().0.as_str());
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running teshi-desktop");
}

fn init_logging() {
    // 日志滚动写入 %APPDATA%/teshi-desktop/logs/，解析数据目录失败时回退到系统临时目录。
    let log_dir = crate::app_data::app_data_dir()
        .map(|p| p.join("logs"))
        .unwrap_or_else(|_| std::env::temp_dir().join("teshi-desktop-logs"));
    let file_appender = tracing_appender::rolling::daily(log_dir, "teshi-desktop.log");
    let (writer, guard) = tracing_appender::non_blocking(file_appender);
    // non-blocking writer 的 guard 必须存活至进程结束，否则缓冲日志会丢失。
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
