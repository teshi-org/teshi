//! Application data directory helpers (recent, settings, logs).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const RECENT_MAX: usize = 10;

/// Minimum window width for desktop shells.
pub const MIN_WINDOW_WIDTH: u32 = 1280;
/// Minimum window height for desktop shells.
pub const MIN_WINDOW_HEIGHT: u32 = 720;

/// Returns `(width, height)` only when both dimensions meet the configured minimum.
pub fn validated_window_size(width: u32, height: u32) -> Option<(u32, u32)> {
    if width >= MIN_WINDOW_WIDTH && height >= MIN_WINDOW_HEIGHT {
        Some((width, height))
    } else {
        None
    }
}

/// Persisted UI preferences (window geometry is handled by the window-state plugin on desktop).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    /// Legacy window width migrated to the window-state plugin on first launch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_width: Option<u32>,
    /// Legacy window height migrated to the window-state plugin on first launch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_height: Option<u32>,
    pub last_project_parent: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RecentFile {
    paths: Vec<String>,
}

/// Returns the Teshi app data directory.
///
/// Resolution order:
/// 1. `TESHI_APP_DATA_DIR` when set and non-empty (tests / custom installs)
/// 2. `%APPDATA%/teshi-desktop` (or XDG equivalent)
pub fn app_data_dir() -> Result<PathBuf> {
    let dir = if let Ok(override_dir) = std::env::var("TESHI_APP_DATA_DIR") {
        let trimmed = override_dir.trim();
        if !trimmed.is_empty() {
            PathBuf::from(trimmed)
        } else {
            default_app_data_dir()?
        }
    } else {
        default_app_data_dir()?
    };
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    fs::create_dir_all(dir.join("logs")).ok();
    Ok(dir)
}

fn default_app_data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir().context("could not resolve data directory")?;
    Ok(base.join("teshi-desktop"))
}

fn recent_path() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("recent.json"))
}

fn settings_path() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("settings.json"))
}

/// Reads the recent project list, pruning missing paths.
pub fn get_recent_projects() -> Result<Vec<String>> {
    let path = recent_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)?;
    let mut file: RecentFile = serde_json::from_str(&content).unwrap_or_default();
    file.paths.retain(|p| Path::new(p).exists());
    Ok(file.paths)
}

/// Adds a project path to the recent list (LRU, max 10).
pub fn add_recent_project(path: &Path) -> Result<Vec<String>> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let entry = canonical.to_string_lossy().into_owned();
    let mut paths = get_recent_projects()?;
    paths.retain(|p| p != &entry);
    paths.insert(0, entry);
    paths.truncate(RECENT_MAX);
    let file = RecentFile {
        paths: paths.clone(),
    };
    fs::write(recent_path()?, serde_json::to_string_pretty(&file)?)?;
    Ok(paths)
}

/// Loads settings from disk.
pub fn load_settings() -> Result<AppSettings> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(AppSettings::default());
    }
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content).unwrap_or_default())
}

/// Persists settings to disk.
pub fn save_settings(settings: &AppSettings) -> Result<()> {
    fs::write(settings_path()?, serde_json::to_string_pretty(settings)?)?;
    Ok(())
}

/// Updates last project parent used as Open dialog default directory.
pub fn remember_project_parent(project_root: &Path) -> Result<()> {
    let mut settings = load_settings()?;
    settings.last_project_parent = project_root
        .parent()
        .map(|p| p.to_string_lossy().into_owned());
    save_settings(&settings)
}

/// Default directory for the Open Project dialog.
pub fn open_dialog_default_dir() -> Option<PathBuf> {
    if let Ok(settings) = load_settings() {
        if let Some(parent) = settings.last_project_parent {
            let path = PathBuf::from(parent);
            if path.is_dir() {
                return Some(path);
            }
        }
    }
    dirs::home_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validated_window_size_rejects_zero_and_sub_minimum() {
        assert!(validated_window_size(0, 0).is_none());
        assert!(validated_window_size(1600, 0).is_none());
        assert!(validated_window_size(0, 900).is_none());
        assert!(validated_window_size(MIN_WINDOW_WIDTH - 1, MIN_WINDOW_HEIGHT).is_none());
        assert!(validated_window_size(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT - 1).is_none());
    }

    #[test]
    fn validated_window_size_accepts_minimum_and_above() {
        assert_eq!(
            validated_window_size(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT),
            Some((MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT))
        );
        assert_eq!(validated_window_size(1920, 1080), Some((1920, 1080)));
    }
}
