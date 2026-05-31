//! Gherkin render command for Panel1.

use std::fs;
use std::path::PathBuf;

use tauri::{AppHandle, Emitter, State};
use teshi_gherkin::render_feature;
use teshi_gherkin::FeatureRenderPayload;

use crate::project::ProjectState;
use crate::watcher::FileWatcherState;

#[tauri::command]
pub async fn render_feature_cmd(
    app: AppHandle,
    path: String,
    state: State<'_, ProjectState>,
    watcher: State<'_, FileWatcherState>,
) -> Result<FeatureRenderPayload, String> {
    let file_path = PathBuf::from(&path);
    let project_root = state
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    let content = fs::read_to_string(&file_path).map_err(|e| e.to_string())?;
    let payload = render_feature(&content, file_path, &project_root);

    watcher
        .watch(&payload.path, app.clone())
        .map_err(|e| e.to_string())?;

    Ok(payload)
}

pub fn emit_feature_refresh(app: &AppHandle, path: &PathBuf, project_root: &PathBuf) {
    if let Ok(content) = fs::read_to_string(path) {
        let payload = render_feature(&content, path.clone(), project_root);
        let _ = app.emit("feature-refreshed", payload);
    }
}
