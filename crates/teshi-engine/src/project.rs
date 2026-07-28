//! Project open, directory listing, and teardown coordination.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use teshi_core::authoring::AuthoringArtifacts;

use crate::app_data::{add_recent_project, remember_project_parent};
use crate::authoring::load_authoring_artifacts;
use crate::locator::start_locator_watch;
use crate::TeshiEngine;

/// Shared runtime state for the opened project.
pub struct ProjectState {
    pub root: Mutex<Option<PathBuf>>,
    pub browser_active: Mutex<bool>,
    pub terminal_active: Mutex<bool>,
    /// Loaded requirement/test-point artifacts when present; `None` for feature-only projects.
    pub authoring: Mutex<Option<AuthoringArtifacts>>,
}

impl Default for ProjectState {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectState {
    /// Creates an empty project holder.
    pub fn new() -> Self {
        Self {
            root: Mutex::new(None),
            browser_active: Mutex::new(false),
            terminal_active: Mutex::new(false),
            authoring: Mutex::new(None),
        }
    }

    /// Returns true when browser or terminal is still running.
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

/// Returns whether project switch/teardown is allowed without confirmation.
pub fn check_project_switch_allowed(rt: &TeshiEngine) -> bool {
    !rt.project.is_busy()
}

/// Opens a project directory and resets runtime subsystems.
pub async fn open_project(rt: Arc<TeshiEngine>, path: String) -> Result<(), String> {
    let path_buf = PathBuf::from(&path);
    if !path_buf.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    let canonical = path_buf
        .canonicalize()
        .map_err(|e| format!("invalid project path: {e}"))?;

    // If opening the same project that's already open, skip stopping
    // the sidecar and terminal so that in-flight sessions (browser bridge,
    // terminal PTY) survive the re-open. The e2eOpenProject flow always
    // calls teardownRuntime before open_project, but replay bindings that
    // use open_project as a setup step should not kill existing sessions
    // when the project hasn't changed.
    {
        let guard = rt.project.root.lock().unwrap();
        if guard.as_ref().map(|p| dunce::simplified(p)) == Some(dunce::simplified(&canonical)) {
            *rt.project.browser_active.lock().unwrap() = false;
            *rt.project.terminal_active.lock().unwrap() = false;
            rt.events
                .emit("project-changed", canonical.to_string_lossy().to_string());
            return Ok(());
        }
    }

    rt.sidecar.stop().await.map_err(|e| e.to_string())?;
    rt.terminal.stop().map_err(|e| e.to_string())?;
    rt.watcher.clear().map_err(|e| e.to_string())?;
    rt.locator_watcher.clear().map_err(|e| e.to_string())?;

    *rt.project.root.lock().unwrap() = Some(canonical.clone());
    *rt.project.browser_active.lock().unwrap() = false;
    *rt.project.terminal_active.lock().unwrap() = false;

    match load_authoring_artifacts(&canonical) {
        Ok(result) => {
            *rt.project.authoring.lock().unwrap() = result.artifacts;
        }
        Err(error) => {
            tracing::warn!(%error, "could not load authoring artifacts");
            *rt.project.authoring.lock().unwrap() = None;
        }
    }

    // User-level recent-project preferences are ancillary state. A read-only
    // profile, sandbox, or unavailable data directory must not prevent the
    // project (and therefore the daemon) from opening.
    if let Err(error) = remember_project_parent(&canonical) {
        tracing::warn!(%error, "could not persist the last project parent");
    }
    let recent = match add_recent_project(&canonical) {
        Ok(recent) => recent,
        Err(error) => {
            tracing::warn!(%error, "could not persist the recent project list");
            vec![canonical.to_string_lossy().into_owned()]
        }
    };

    rt.events.emit("recent-loaded", recent);
    rt.events
        .emit("project-changed", canonical.to_string_lossy().to_string());
    start_locator_watch(&rt.locator_watcher, &canonical, Arc::clone(&rt))?;
    Ok(())
}

/// Lists directory entries under the open project root.
pub fn list_dir(rt: &TeshiEngine, path: String) -> Result<Vec<DirEntry>, String> {
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

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

/// Returns loaded authoring artifacts for the open project, if any.
pub fn get_authoring_artifacts(rt: &TeshiEngine) -> Option<AuthoringArtifacts> {
    rt.project.authoring.lock().unwrap().clone()
}

/// Returns the canonical project root path when a project is open.
pub fn get_project_root(rt: &TeshiEngine) -> Option<String> {
    rt.project
        .root
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Sets whether the browser sidecar is considered active.
pub fn set_browser_active(rt: &TeshiEngine, active: bool) {
    *rt.project.browser_active.lock().unwrap() = active;
}

/// Sets whether the embedded terminal session is considered active.
pub fn set_terminal_active(rt: &TeshiEngine, active: bool) {
    *rt.project.terminal_active.lock().unwrap() = active;
}

/// Stops browser, terminal, and file watchers without clearing the project root.
pub async fn teardown_runtime(rt: &TeshiEngine) -> Result<(), String> {
    rt.terminal.stop().map_err(|e| e.to_string())?;
    rt.watcher.clear().map_err(|e| e.to_string())?;
    rt.locator_watcher.clear().map_err(|e| e.to_string())?;
    *rt.project.browser_active.lock().unwrap() = false;
    *rt.project.terminal_active.lock().unwrap() = false;
    Ok(())
}
