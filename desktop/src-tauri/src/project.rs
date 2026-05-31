//! Project open, directory listing, and teardown coordination.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::app_data::{add_recent_project, remember_project_parent};
use crate::locator::{start_locator_watch, LocatorWatcherState};
use crate::sidecar::SidecarState;
use crate::terminal::TerminalState;
use crate::watcher::FileWatcherState;

/// Shared runtime state for the opened project.
pub struct ProjectState {
    pub root: Mutex<Option<PathBuf>>,
    pub browser_active: Mutex<bool>,
    pub terminal_active: Mutex<bool>,
}

impl ProjectState {
    pub fn new() -> Self {
        Self {
            root: Mutex::new(None),
            browser_active: Mutex::new(false),
            terminal_active: Mutex::new(false),
        }
    }

    pub fn is_busy(&self) -> bool {
        *self.browser_active.lock().unwrap() || *self.terminal_active.lock().unwrap()
    }
}

/// One entry in a lazy-loaded directory listing.
#[derive(Debug, Clone, Serialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_feature: bool,
}

#[tauri::command]
pub async fn check_project_switch_allowed(state: State<'_, ProjectState>) -> Result<bool, String> {
    Ok(!state.is_busy())
}

#[tauri::command]
pub async fn open_project(
    app: AppHandle,
    path: String,
    state: State<'_, ProjectState>,
    sidecar: State<'_, SidecarState>,
    terminal: State<'_, TerminalState>,
    watcher: State<'_, FileWatcherState>,
    locator_watcher: State<'_, LocatorWatcherState>,
) -> Result<(), String> {
    let path_buf = PathBuf::from(&path);
    if !path_buf.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    let canonical = path_buf
        .canonicalize()
        .map_err(|e| format!("invalid project path: {e}"))?;

    sidecar.stop().await.map_err(|e| e.to_string())?;
    terminal.stop().map_err(|e| e.to_string())?;
    watcher.clear().map_err(|e| e.to_string())?;
    locator_watcher.clear().map_err(|e| e.to_string())?;

    *state.root.lock().unwrap() = Some(canonical.clone());
    *state.browser_active.lock().unwrap() = false;
    *state.terminal_active.lock().unwrap() = false;

    remember_project_parent(&canonical).map_err(|e| e.to_string())?;
    add_recent_project(&canonical).map_err(|e| e.to_string())?;

    app.emit("project-changed", canonical.to_string_lossy().to_string())
        .map_err(|e| e.to_string())?;
    start_locator_watch(&locator_watcher, &canonical, app)?;
    Ok(())
}

#[tauri::command]
pub fn list_dir(path: String, state: State<'_, ProjectState>) -> Result<Vec<DirEntry>, String> {
    let project_root = state
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    // 先 canonicalize 再做包含校验，避免通过 `..` 等相对路径逃逸项目根目录。
    let dir = PathBuf::from(&path)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let root = project_root.canonicalize().map_err(|e| e.to_string())?;
    if !dir.starts_with(&root) {
        return Err("path outside project root".into());
    }
    if !dir.is_dir() {
        return Err("not a directory".into());
    }

    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let meta = entry.metadata().map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path_str = entry.path().to_string_lossy().into_owned();
        let is_feature = !meta.is_dir() && name.ends_with(".feature");
        entries.push(DirEntry {
            name,
            path: path_str,
            is_dir: meta.is_dir(),
            is_feature,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

#[tauri::command]
pub fn get_project_root(state: State<'_, ProjectState>) -> Option<String> {
    state
        .root
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn set_browser_active(active: bool, state: State<'_, ProjectState>) {
    *state.browser_active.lock().unwrap() = active;
}

#[tauri::command]
pub fn set_terminal_active(active: bool, state: State<'_, ProjectState>) {
    *state.terminal_active.lock().unwrap() = active;
}

#[tauri::command]
pub async fn teardown_runtime(
    sidecar: State<'_, SidecarState>,
    terminal: State<'_, TerminalState>,
    watcher: State<'_, FileWatcherState>,
    locator_watcher: State<'_, LocatorWatcherState>,
    state: State<'_, ProjectState>,
) -> Result<(), String> {
    sidecar.stop().await.map_err(|e| e.to_string())?;
    terminal.stop().map_err(|e| e.to_string())?;
    watcher.clear().map_err(|e| e.to_string())?;
    locator_watcher.clear().map_err(|e| e.to_string())?;
    *state.browser_active.lock().unwrap() = false;
    *state.terminal_active.lock().unwrap() = false;
    Ok(())
}
