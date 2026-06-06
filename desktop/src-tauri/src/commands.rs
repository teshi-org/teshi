//! Thin Tauri command adapters over `teshi-runtime`.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use teshi_gherkin::FeatureRenderPayload;
use teshi_runtime::{
    abandon_pending_locator, check_project_switch_allowed, confirm_locator, get_active_step,
    get_pending_locator, get_project_root as runtime_project_root, get_recent_projects,
    highlight_locator, list_dir as runtime_list_dir, load_project_settings,
    open_dialog_default_dir, open_project as runtime_open_project, reject_locator,
    render_feature as runtime_render_feature, resize_terminal as runtime_resize,
    set_browser_active, set_terminal_active, spawn_terminal as runtime_spawn_terminal,
    start_browser_sidecar as runtime_start_browser, step_binding_statuses,
    stop_browser_sidecar as runtime_stop_browser, stop_terminal as runtime_stop_terminal,
    sync_active_step, teardown_runtime as runtime_teardown, unbind_step,
    write_terminal as runtime_write_terminal, ActiveStep, BrowserError, BrowserMode,
    BrowserStartResult, DirEntry, PendingLocator, ProjectSettings, StepBinding, StepBindingStatus,
    TeshiRuntime,
};

#[tauri::command]
pub async fn check_project_switch_allowed_cmd(
    rt: State<'_, Arc<TeshiRuntime>>,
) -> Result<bool, String> {
    Ok(check_project_switch_allowed(&rt))
}

#[tauri::command]
pub async fn open_project(rt: State<'_, Arc<TeshiRuntime>>, path: String) -> Result<(), String> {
    runtime_open_project(Arc::clone(&rt), path).await
}

#[tauri::command]
pub fn list_dir(rt: State<'_, Arc<TeshiRuntime>>, path: String) -> Result<Vec<DirEntry>, String> {
    runtime_list_dir(&rt, path)
}

#[tauri::command]
pub fn get_project_root(rt: State<'_, Arc<TeshiRuntime>>) -> Option<String> {
    runtime_project_root(&rt)
}

#[tauri::command]
pub async fn render_feature_cmd(
    rt: State<'_, Arc<TeshiRuntime>>,
    path: String,
) -> Result<FeatureRenderPayload, String> {
    runtime_render_feature(&rt, path)
}

#[tauri::command]
pub async fn sync_active_step_cmd(
    rt: State<'_, Arc<TeshiRuntime>>,
    feature_path: String,
    step_line: u32,
) -> Result<ActiveStep, String> {
    sync_active_step(&rt, feature_path, step_line).await
}

#[tauri::command]
pub fn get_active_step_cmd(rt: State<'_, Arc<TeshiRuntime>>) -> Result<Option<ActiveStep>, String> {
    get_active_step(&rt)
}

#[tauri::command]
pub fn get_pending_locator_cmd(
    rt: State<'_, Arc<TeshiRuntime>>,
) -> Result<Option<PendingLocator>, String> {
    get_pending_locator(&rt)
}

#[tauri::command]
pub fn get_step_binding_statuses_cmd(
    rt: State<'_, Arc<TeshiRuntime>>,
    feature_path: String,
) -> Result<Vec<StepBindingStatus>, String> {
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;
    step_binding_statuses(&project_root, &feature_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn confirm_locator_cmd(
    rt: State<'_, Arc<TeshiRuntime>>,
    candidate_rank: u32,
    edited_value: Option<String>,
) -> Result<(), String> {
    confirm_locator(&rt, candidate_rank, edited_value).await
}

#[tauri::command]
pub async fn reject_locator_cmd(rt: State<'_, Arc<TeshiRuntime>>) -> Result<(), String> {
    reject_locator(&rt).await
}

#[tauri::command]
pub async fn highlight_locator_cmd(
    rt: State<'_, Arc<TeshiRuntime>>,
    selector: String,
) -> Result<(), String> {
    highlight_locator(&rt, selector).await
}

#[tauri::command]
pub async fn abandon_pending_locator_cmd(rt: State<'_, Arc<TeshiRuntime>>) -> Result<(), String> {
    abandon_pending_locator(&rt).await
}

#[tauri::command]
pub async fn unbind_step_cmd(
    rt: State<'_, Arc<TeshiRuntime>>,
    feature_path: String,
    step_line: u32,
) -> Result<Option<StepBinding>, String> {
    unbind_step(&rt, feature_path, step_line).await
}

#[tauri::command]
pub fn get_project_settings_cmd(
    rt: State<'_, Arc<TeshiRuntime>>,
) -> Result<ProjectSettings, String> {
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;
    load_project_settings(&project_root).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_browser_sidecar(
    rt: State<'_, Arc<TeshiRuntime>>,
    mode: String,
) -> Result<BrowserStartResult, BrowserError> {
    let mode = match mode.as_str() {
        "chrome" => BrowserMode::Chrome,
        "winapp" => BrowserMode::WinApp,
        _ => BrowserMode::Embedded,
    };
    runtime_start_browser(Arc::clone(&rt), mode).await
}

#[tauri::command]
pub async fn stop_browser_sidecar(rt: State<'_, Arc<TeshiRuntime>>) -> Result<(), String> {
    runtime_stop_browser(&rt).await
}

#[tauri::command]
pub async fn spawn_terminal(
    rt: State<'_, Arc<TeshiRuntime>>,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    runtime_spawn_terminal(Arc::clone(&rt), cols, rows).await
}

#[tauri::command]
pub fn stop_terminal(rt: State<'_, Arc<TeshiRuntime>>) -> Result<(), String> {
    runtime_stop_terminal(&rt)
}

#[tauri::command]
pub fn resize_terminal(
    rt: State<'_, Arc<TeshiRuntime>>,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    runtime_resize(&rt, cols, rows)
}

#[tauri::command]
pub fn write_terminal(rt: State<'_, Arc<TeshiRuntime>>, data: String) -> Result<(), String> {
    runtime_write_terminal(&rt, data)
}

#[tauri::command]
pub async fn teardown_runtime(rt: State<'_, Arc<TeshiRuntime>>) -> Result<(), String> {
    runtime_teardown(&rt).await
}

#[tauri::command]
pub fn set_browser_active_cmd(rt: State<'_, Arc<TeshiRuntime>>, active: bool) {
    set_browser_active(&rt, active);
}

#[tauri::command]
pub fn set_terminal_active_cmd(rt: State<'_, Arc<TeshiRuntime>>, active: bool) {
    set_terminal_active(&rt, active);
}

#[tauri::command]
pub fn get_recent_projects_cmd() -> Result<Vec<String>, String> {
    get_recent_projects()
}

/// Opens the native folder picker and returns the selected path.
#[tauri::command]
pub async fn open_project_dir(app: AppHandle) -> Result<Option<String>, String> {
    let default = open_dialog_default_dir();
    let picked = app
        .dialog()
        .file()
        .set_title("Open Project")
        .set_directory(default.unwrap_or_else(|| PathBuf::from(".")))
        .blocking_pick_folder();

    Ok(picked.map(|p| p.to_string()))
}

/// Read a text file from disk (used for `.teshi/cdp-endpoint.json` polling).
#[tauri::command]
pub fn read_text_file(path: String) -> Result<String, String> {
    fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))
}

/// Confirms destructive switch/exit when runtime is active.
#[tauri::command]
pub async fn confirm_teardown(
    rt: State<'_, Arc<TeshiRuntime>>,
    app: AppHandle,
) -> Result<bool, String> {
    if check_project_switch_allowed(&rt) {
        return Ok(true);
    }
    let answer = app
        .dialog()
        .message("Browser/Terminal is running. Continuing will stop them.")
        .title("Confirm")
        .buttons(MessageDialogButtons::OkCancel)
        .kind(MessageDialogKind::Warning)
        .blocking_show();

    Ok(answer)
}
