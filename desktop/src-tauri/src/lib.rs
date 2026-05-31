mod app_data;
mod gherkin_cmd;
mod project;
mod sidecar;
mod terminal;
mod watcher;

use tauri::{Emitter, Manager};

use crate::app_data::{get_recent_projects, load_settings, save_settings};
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
            if let Ok(recent) = get_recent_projects() {
                let _ = app.emit("recent-loaded", recent);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running teshi-desktop");
}

fn init_logging() {
    let _ = tracing_subscriber::fmt().with_env_filter("info").try_init();
}

#[tauri::command]
fn save_window_settings(width: u32, height: u32) -> Result<(), String> {
    let mut settings = load_settings().map_err(|e| e.to_string())?;
    settings.window_width = Some(width);
    settings.window_height = Some(height);
    save_settings(&settings).map_err(|e| e.to_string())?;
    Ok(())
}
