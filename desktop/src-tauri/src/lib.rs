mod app_data;
mod gherkin_cmd;
mod project;
mod sidecar;
mod terminal;
mod watcher;

use tauri::{Emitter, Manager};

use crate::app_data::{
    get_recent_projects, load_settings, save_settings, validated_window_size,
    DEFAULT_WINDOW_HEIGHT, DEFAULT_WINDOW_WIDTH,
};
use crate::gherkin_cmd::render_feature_cmd;
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();

    tauri::Builder::default()
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
        .invoke_handler(tauri::generate_handler![
            open_project_dir,
            open_project,
            get_recent_projects_cmd,
            list_dir,
            get_project_root,
            render_feature_cmd,
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
            save_window_settings,
        ])
        .setup(|app| {
            // Restore persisted window size; ignore corrupt/zero values from early resize events.
            if let Some(window) = app.get_webview_window("main") {
                let size = load_settings()
                    .ok()
                    .and_then(|settings| {
                        validated_window_size(settings.window_width?, settings.window_height?)
                    })
                    .unwrap_or((DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT));
                let _ = window.set_size(tauri::LogicalSize::new(size.0 as f64, size.1 as f64));
            }
            if let Ok(recent) = get_recent_projects() {
                let _ = app.emit("recent-loaded", recent);
            }
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

#[tauri::command]
fn save_window_settings(width: u32, height: u32) -> Result<(), String> {
    let Some((width, height)) = validated_window_size(width, height) else {
        // Ignore transient zero-size resize events during startup or minimize.
        return Ok(());
    };
    let mut settings = load_settings().map_err(|e| e.to_string())?;
    settings.window_width = Some(width);
    settings.window_height = Some(height);
    save_settings(&settings).map_err(|e| e.to_string())?;
    Ok(())
}
