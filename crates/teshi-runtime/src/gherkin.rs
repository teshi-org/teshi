//! Gherkin render for the structured feature panel.

use std::fs;
use std::path::{Path, PathBuf};

use teshi_gherkin::render_feature as render_feature_content;
use teshi_gherkin::FeatureRenderPayload;

use crate::events::RuntimeEvents;
use crate::TeshiRuntime;

/// Renders a `.feature` file and starts watching it for changes.
pub fn render_feature(rt: &TeshiRuntime, path: String) -> Result<FeatureRenderPayload, String> {
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    let file_path = canonical_child(Path::new(&path), &project_root)?;
    if file_path.extension().and_then(|s| s.to_str()) != Some("feature") {
        return Err("not a .feature file".into());
    }

    let content = fs::read_to_string(&file_path).map_err(|e| e.to_string())?;
    let payload = render_feature_content(&content, file_path, &project_root);

    rt.watcher
        .watch(&payload.path, &project_root, rt.events.clone())
        .map_err(|e| e.to_string())?;

    Ok(payload)
}

fn canonical_child(path: &Path, root: &Path) -> Result<PathBuf, String> {
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if !canonical.starts_with(&root) {
        return Err("path outside project root".into());
    }
    Ok(canonical)
}

/// Re-reads a feature file and emits `feature-refreshed`.
pub fn emit_feature_refresh(events: &RuntimeEvents, path: &Path, project_root: &Path) {
    if let Ok(content) = fs::read_to_string(path) {
        let payload = render_feature_content(&content, path.to_path_buf(), project_root);
        events.emit("feature-refreshed", payload);
    }
}
