//! Per-project settings under `.teshi/settings.json`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Default auto-confirm delay for locator proposals (seconds); `0` disables.
pub const DEFAULT_LOCATOR_AUTO_CONFIRM_SEC: u64 = 60;

/// Project-local teshi settings (`.teshi/settings.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSettings {
    /// Seconds to wait before auto-confirming a pending locator; `0` = manual only.
    #[serde(default = "default_locator_auto_confirm_sec")]
    pub locator_auto_confirm_sec: u64,
}

fn default_locator_auto_confirm_sec() -> u64 {
    DEFAULT_LOCATOR_AUTO_CONFIRM_SEC
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            locator_auto_confirm_sec: DEFAULT_LOCATOR_AUTO_CONFIRM_SEC,
        }
    }
}

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
