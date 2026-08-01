//! Application data directory helpers (recent, settings, logs).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const RECENT_MAX: usize = 10;
const DESKTOP_MIGRATION_MARKER: &str = ".migrated-from-teshi-desktop";
const LEGACY_DESKTOP_DIR_NAME: &str = "teshi-desktop";
const APP_DATA_DIR_NAME: &str = "teshi";

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
/// 2. `%APPDATA%/teshi` (or XDG data-home equivalent)
///
/// When using the default path, performs a one-time copy from the legacy
/// `teshi-desktop` directory if present.
pub fn app_data_dir() -> Result<PathBuf> {
    let (dir, is_default) = resolve_app_data_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    if is_default {
        let legacy = legacy_desktop_app_data_dir()?;
        ensure_migrated_from_teshi_desktop_at(&dir, legacy.as_deref())?;
    }
    fs::create_dir_all(dir.join("logs")).ok();
    Ok(dir)
}

fn resolve_app_data_dir() -> Result<(PathBuf, bool)> {
    if let Ok(override_dir) = std::env::var("TESHI_APP_DATA_DIR") {
        let trimmed = override_dir.trim();
        if !trimmed.is_empty() {
            return Ok((PathBuf::from(trimmed), false));
        }
    }
    Ok((default_app_data_dir()?, true))
}

fn default_app_data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir().context("could not resolve data directory")?;
    Ok(base.join(APP_DATA_DIR_NAME))
}

fn legacy_desktop_app_data_dir() -> Result<Option<PathBuf>> {
    let base = dirs::data_dir().context("could not resolve data directory")?;
    let legacy = base.join(LEGACY_DESKTOP_DIR_NAME);
    Ok(if legacy.is_dir() { Some(legacy) } else { None })
}

/// One-time copy of legacy `teshi-desktop` app data into `new_root`.
///
/// Copies `model-profiles/`, `llm-config.json`, `settings.json`, and
/// `recent.json` when present. Never deletes the legacy directory.
///
/// # Errors
///
/// Returns an error when directory or file I/O fails.
pub fn ensure_migrated_from_teshi_desktop_at(
    new_root: &Path,
    legacy_root: Option<&Path>,
) -> Result<()> {
    fs::create_dir_all(new_root).with_context(|| format!("create {}", new_root.display()))?;
    let marker = new_root.join(DESKTOP_MIGRATION_MARKER);
    if marker.exists() {
        return Ok(());
    }

    if let Some(legacy) = legacy_root {
        if legacy.is_dir() && legacy != new_root {
            copy_if_exists(legacy, new_root, "model-profiles")?;
            copy_file_if_exists(legacy, new_root, "llm-config.json")?;
            copy_file_if_exists(legacy, new_root, "settings.json")?;
            copy_file_if_exists(legacy, new_root, "recent.json")?;
        }
    }

    fs::write(&marker, b"1").with_context(|| format!("write {}", marker.display()))?;
    Ok(())
}

fn copy_if_exists(src_root: &Path, dst_root: &Path, name: &str) -> Result<()> {
    let src = src_root.join(name);
    if !src.exists() {
        return Ok(());
    }
    let dst = dst_root.join(name);
    if dst.is_dir() {
        // Destination directory already exists: merge missing files from source so that
        // an empty or partially-filled directory gets completed. Files already present
        // in dst are never overwritten, preserving any data the user has set up.
        merge_dir_into(&src, &dst)?;
        return Ok(());
    }
    copy_dir_recursive(&src, &dst)
}

/// Copy files from `src` into `dst`, skipping files that already exist in `dst`.
fn merge_dir_into(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("create {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("read {}", src.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if file_type.is_dir() {
            merge_dir_into(&from, &to)?;
        } else if file_type.is_file() && !to.exists() {
            fs::copy(&from, &to)
                .with_context(|| format!("copy {} -> {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

fn copy_file_if_exists(src_root: &Path, dst_root: &Path, name: &str) -> Result<()> {
    let src = src_root.join(name);
    if !src.is_file() {
        return Ok(());
    }
    let dst = dst_root.join(name);
    if dst.exists() {
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::copy(&src, &dst).with_context(|| format!("copy {} -> {}", src.display(), dst.display()))?;
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("create {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("read {}", src.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to)
                .with_context(|| format!("copy {} -> {}", from.display(), to.display()))?;
        }
    }
    Ok(())
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
    use tempfile::TempDir;

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

    #[test]
    fn test_migrate_from_teshi_desktop_copies_profiles_once() {
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("teshi-desktop");
        let neu = tmp.path().join("teshi");
        let profiles = legacy.join("model-profiles");
        fs::create_dir_all(&profiles).unwrap();
        fs::write(
            profiles.join("abc.json"),
            r#"{"id":"abc","name":"Desktop","provider":"openai","model_id":"gpt-4o","api_key":"sk-desk"}"#,
        )
        .unwrap();
        fs::write(
            legacy.join("settings.json"),
            r#"{"last_project_parent":"/x"}"#,
        )
        .unwrap();

        ensure_migrated_from_teshi_desktop_at(&neu, Some(&legacy)).unwrap();
        assert!(neu.join("model-profiles/abc.json").is_file());
        assert!(neu.join("settings.json").is_file());
        assert!(neu.join(DESKTOP_MIGRATION_MARKER).is_file());

        // Second run must not duplicate or error.
        fs::write(
            profiles.join("extra.json"),
            r#"{"id":"extra","name":"Later","provider":"openai","model_id":"x","api_key":""}"#,
        )
        .unwrap();
        ensure_migrated_from_teshi_desktop_at(&neu, Some(&legacy)).unwrap();
        assert!(!neu.join("model-profiles/extra.json").exists());
    }

    #[test]
    fn test_migrate_merges_missing_profiles_when_dst_dir_exists() {
        // When dst model-profiles/ already has some profiles, legacy profiles that are
        // missing from dst must be merged in (not skipped). Existing dst files are kept.
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("teshi-desktop");
        let neu = tmp.path().join("teshi");
        fs::create_dir_all(legacy.join("model-profiles")).unwrap();
        fs::write(
            legacy.join("model-profiles/old.json"),
            r#"{"id":"old","name":"Old","provider":"openai","model_id":"a","api_key":"k"}"#,
        )
        .unwrap();
        // dst already has a different profile.
        fs::create_dir_all(neu.join("model-profiles")).unwrap();
        fs::write(
            neu.join("model-profiles/keep.json"),
            r#"{"id":"keep","name":"Keep","provider":"openai","model_id":"b","api_key":"k2"}"#,
        )
        .unwrap();

        ensure_migrated_from_teshi_desktop_at(&neu, Some(&legacy)).unwrap();
        // Existing profile must be preserved.
        assert!(neu.join("model-profiles/keep.json").is_file());
        // Missing legacy profile must be merged in.
        assert!(neu.join("model-profiles/old.json").is_file());
        assert!(neu.join(DESKTOP_MIGRATION_MARKER).is_file());
    }

    #[test]
    fn test_migrate_copies_into_empty_dst_profiles_dir() {
        // An empty dst model-profiles/ must receive all legacy profiles.
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("teshi-desktop");
        let neu = tmp.path().join("teshi");
        fs::create_dir_all(legacy.join("model-profiles")).unwrap();
        fs::write(
            legacy.join("model-profiles/abc.json"),
            r#"{"id":"abc","name":"Leg","provider":"openai","model_id":"gpt-4o","api_key":"sk-l"}"#,
        )
        .unwrap();
        // Empty dst dir exists.
        fs::create_dir_all(neu.join("model-profiles")).unwrap();

        ensure_migrated_from_teshi_desktop_at(&neu, Some(&legacy)).unwrap();
        assert!(
            neu.join("model-profiles/abc.json").is_file(),
            "empty dst must receive legacy profiles"
        );
        assert!(neu.join(DESKTOP_MIGRATION_MARKER).is_file());
    }
}
