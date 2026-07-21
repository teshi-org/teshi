//! Per-project settings I/O (loading from `.teshi/settings.json`).
//! Pure DTOs live in `teshi-core::project_settings`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

pub use teshi_core::project_settings::{ProjectSettings, DEFAULT_LOCATOR_AUTO_CONFIRM_SEC};

fn settings_path(project_root: &Path) -> std::path::PathBuf {
    project_root.join(".teshi").join("settings.json")
}

/// Loads project settings, returning defaults when the file is missing.
///
/// # Errors
///
/// Returns an error when the file exists but cannot be parsed.
pub fn load_project_settings(project_root: &Path) -> Result<ProjectSettings> {
    let path = settings_path(project_root);
    if !path.is_file() {
        return Ok(ProjectSettings::default());
    }
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}
