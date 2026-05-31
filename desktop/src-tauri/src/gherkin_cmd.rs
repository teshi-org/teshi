//! Gherkin render command for Panel1.

use std::fs;
use std::path::{Path, PathBuf};

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
    let project_root = state
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    // 校验请求路径必须落在项目根目录内，且确实是 .feature 文件，
    // 防止前端通过 IPC 读取项目外的任意本地文件。
    let file_path = canonical_child(Path::new(&path), &project_root)?;
    if file_path.extension().and_then(|s| s.to_str()) != Some("feature") {
        return Err("not a .feature file".into());
    }

    let content = fs::read_to_string(&file_path).map_err(|e| e.to_string())?;
    let payload = render_feature(&content, file_path, &project_root);

    watcher
        .watch(&payload.path, app.clone())
        .map_err(|e| e.to_string())?;

    Ok(payload)
}

/// 将 `path` 规范化后校验其位于 `root` 之内，返回规范化后的绝对路径。
fn canonical_child(path: &Path, root: &Path) -> Result<PathBuf, String> {
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if !canonical.starts_with(&root) {
        return Err("path outside project root".into());
    }
    Ok(canonical)
}

pub fn emit_feature_refresh(app: &AppHandle, path: &Path, project_root: &Path) {
    if let Ok(content) = fs::read_to_string(path) {
        let payload = render_feature(&content, path.to_path_buf(), project_root);
        let _ = app.emit("feature-refreshed", payload);
    }
}
