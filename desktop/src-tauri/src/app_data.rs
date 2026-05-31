//! Application data directory helpers (recent, settings, logs).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const RECENT_MAX: usize = 10;

/// Persisted window and UI preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub last_project_parent: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RecentFile {
    paths: Vec<String>,
}

/// Returns `%APPDATA%/teshi-desktop` (or XDG equivalent).
pub fn app_data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir().context("could not resolve data directory")?;
    let dir = base.join("teshi-desktop");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    fs::create_dir_all(dir.join("logs")).ok();
    Ok(dir)
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
