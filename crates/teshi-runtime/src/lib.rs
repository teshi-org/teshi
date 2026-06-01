//! Shared BDD runtime for teshi desktop and web hosts.

mod app_data;
mod events;
mod gherkin;
mod locator;
mod project;
mod sidecar;
mod terminal;
mod venv;
mod watcher;

/// Project Python venv resolution and import preflight (used by sidecar; exposed for tests).
pub mod python_env {
    pub use crate::venv::{
        apply_venv_to_command, build_import_check_command, check_failure_detail,
        import_check_failed_message, is_missing_module_failure, is_untrusted_mount_failure,
        is_uv_managed_venv, is_uv_trampoline_failure, is_uv_trampoline_shim, parse_pyvenv_cfg,
        resolve_project_venv, run_import_preflight, venv_python_failure_hint, ResolvedVenv,
    };
}

pub use app_data::{
    app_data_dir, get_recent_projects as load_recent_projects, load_settings,
    open_dialog_default_dir, save_settings, validated_window_size, AppSettings, MIN_WINDOW_HEIGHT,
    MIN_WINDOW_WIDTH,
};
pub use events::{HostEventCallback, RuntimeEvent, RuntimeEvents};
pub use gherkin::{emit_feature_refresh, render_feature};
pub use locator::{
    abandon_pending_locator, confirm_locator, get_active_step, get_pending_locator, reject_locator,
    resolve_step_context, start_locator_watch, sync_active_step, ActiveStep, HighlightInfo,
    LocatorCandidate, LocatorWatcherState, PendingLocator,
};
pub use project::{
    check_project_switch_allowed, get_project_root, list_dir, open_project, set_browser_active,
    set_terminal_active, teardown_runtime, DirEntry, ProjectState,
};
pub use sidecar::{
    get_recent_projects, send_sidecar_command, start_browser_sidecar, stop_browser_sidecar,
    BrowserError, BrowserMode, BrowserStartResult, SidecarState, CHROME_DISCOVERY_PORT,
};
pub use terminal::{resize_terminal, spawn_terminal, stop_terminal, write_terminal, TerminalState};
pub use watcher::FileWatcherState;

use std::path::PathBuf;
use std::sync::Arc;

/// Configuration required to construct a [`TeshiRuntime`].
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Absolute path to `browser_service.py`.
    pub browser_service_script: PathBuf,
}

/// Central holder for project, terminal, browser sidecar, and event bus state.
pub struct TeshiRuntime {
    pub project: ProjectState,
    pub sidecar: SidecarState,
    pub terminal: TerminalState,
    pub watcher: FileWatcherState,
    pub locator_watcher: LocatorWatcherState,
    pub events: RuntimeEvents,
    pub browser_service_script: PathBuf,
}

impl TeshiRuntime {
    /// Builds a runtime with the given config and optional host event forwarding.
    pub fn new(config: RuntimeConfig, host: Option<HostEventCallback>) -> Arc<Self> {
        Arc::new(Self {
            project: ProjectState::new(),
            sidecar: SidecarState::new(),
            terminal: TerminalState::new(),
            watcher: FileWatcherState::new(),
            locator_watcher: LocatorWatcherState::new(),
            events: RuntimeEvents::new(host),
            browser_service_script: config.browser_service_script,
        })
    }

    /// Emits initial `recent-loaded` if recent projects exist on disk.
    pub fn emit_initial_recent(&self) {
        if let Ok(recent) = load_recent_projects() {
            self.events.emit("recent-loaded", recent);
        }
    }
}

/// Resolves `browser_service.py` from `TESHI_BROWSER_SERVICE`, installed layouts, or dev paths.
pub fn default_browser_service_script() -> PathBuf {
    if let Ok(path) = std::env::var("TESHI_BROWSER_SERVICE") {
        return PathBuf::from(path);
    }

    let mut candidates = vec![];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("share").join("browser_service.py"));
            candidates.push(exe_dir.join("..").join("share").join("browser_service.py"));
            candidates.push(exe_dir.join("resources").join("browser_service.py"));
        }
    }
    candidates.extend([
        PathBuf::from("desktop/src-tauri/resources/browser_service.py"),
        PathBuf::from("../desktop/src-tauri/resources/browser_service.py"),
        PathBuf::from("../../desktop/src-tauri/resources/browser_service.py"),
    ]);

    for path in &candidates {
        if path.is_file() {
            return path.clone();
        }
    }
    candidates[0].clone()
}
