//! Shared BDD runtime for teshi desktop and web hosts.

mod app_data;
mod authoring;
mod browser_agent;
mod daemon;
mod events;
mod fs_util;
mod gherkin;
mod legacy_tui_import;
pub mod llm;
mod llm_anthropic;
pub mod llm_config_store;
mod llm_responses;
mod locator;
pub mod model_profile;
mod project;
mod project_settings;
mod screen;
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
    app_data_dir, ensure_migrated_from_teshi_desktop_at,
    get_recent_projects as load_recent_projects, load_settings, open_dialog_default_dir,
    save_settings, validated_window_size, AppSettings, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
};
pub use authoring::{
    compute_document_revision, load_authoring_artifacts, save_requirement_document_index,
    save_requirement_markdown, save_test_points, AuthoringLoadResult, DEFAULT_REQUIREMENTS_DIR,
    DEFAULT_TESTPOINTS_DIR, REQUIREMENTS_INDEX_FILE,
};
pub use browser_agent::{
    AccessibleElement, BrowserAgentError, BrowserAgentErrorCode, BrowserEvidenceReference,
    BrowserLease, BrowserLeaseSummary, BrowserMetadata, BrowserOperation, BrowserOperationResponse,
    BrowserOperations, BrowserPageSnapshot, BrowserSession, BrowserSessionHealth, BrowserTab,
    BrowserTarget, BrowserWindow, ExtensionIdentity, LocatorContext, LocatorIntent,
    LocatorVerificationStatus, PageContextRevision, PlaywrightLocatorCandidate,
    PlaywrightLocatorKind, PlaywrightLocatorResult, BROWSER_AGENT_SCHEMA_VERSION,
    BROWSER_BROKER_PROTOCOL_VERSION, DEFAULT_BROWSER_LEASE_TTL_SECS,
};
pub use daemon::{
    find_project_root, pick_free_port, remove_daemon_manifest, spawn_daemon_background,
    DaemonManifest, DaemonManifestExt,
};
pub use events::{HostEventCallback, RuntimeEvent, RuntimeEvents};
pub use fs_util::{read_locked, write_atomic};
pub use gherkin::{emit_feature_refresh, rebuild_and_emit_step_index, render_feature};
pub use legacy_tui_import::{
    ensure_tui_legacy_imported_at, legacy_tui_config_dir, map_legacy_provider_id,
};
pub use llm::{call_llm_with_tool, call_llm_with_tool_config, llm_config_from_env};
pub use llm_config_store::{
    effective_llm_config, load_llm_config_public, load_stored_llm_config, save_stored_llm_config,
    to_public, LlmConfigPublic, LlmConfigWrite, StoredLlmConfig,
};
pub use locator::{
    abandon_pending_locator, active_step_mismatch_with_pending, confirm_locator,
    confirm_pending_locator, first_unbound_feature_step, get_active_step, get_pending_locator,
    highlight_locator, list_feature_step_refs, list_step_bindings, normalize_step_text,
    propose_locator, read_active_step, read_pending, reject_locator, reject_pending_locator,
    resolve_step_bindings, resolve_step_context, sanitize_feature_path, start_locator_watch,
    step_binding_statuses, sync_active_step, unbind_step, unbind_step_binding,
    update_binding_locator, wait_for_step_status, write_active_step, ActiveStep, FeatureStepRef,
    HighlightInfo, LocatorCandidate, LocatorPrimary, LocatorWatcherState, PendingLocator,
    StepBinding, StepBindingStatus, StepBindingsFile, StepWaitResult, StepWaitUntil,
};
pub use model_profile::{
    default_base_url_for_provider, delete_profile, effective_api_style, ensure_migrated,
    generate_id, get_profile_public, is_builtin_provider, list_profiles, load_active_profile,
    load_profile, model_profiles_dir, profile_to_llm_config, read_active_id, resolve_base_url,
    save_profile, set_active_id, to_public_profile, validate_profile, validate_profile_id,
    ApiStyle, ModelProfile, ModelProfileList, ModelProfilePublic, DEFAULT_BASE_URL_ANTHROPIC,
    DEFAULT_BASE_URL_DEEPSEEK, DEFAULT_BASE_URL_OPENAI, PROVIDER_ANTHROPIC,
    PROVIDER_DEEPSEEK_OPENAI, PROVIDER_OPENAI,
};
pub use project::{
    check_project_switch_allowed, get_authoring_artifacts, get_project_root, list_dir,
    open_project, set_browser_active, set_terminal_active, teardown_runtime, DirEntry,
    ProjectState,
};
pub use project_settings::{
    load_project_settings, ProjectSettings, DEFAULT_LOCATOR_AUTO_CONFIRM_SEC,
    DEFAULT_PLAYWRIGHT_TEST_ID_ATTRIBUTE,
};
pub use screen::{Cell, Color, ProcessState, ScreenGrid};
pub use sidecar::{
    get_recent_projects, send_sidecar_command, send_sidecar_command_with_timeout,
    start_browser_sidecar, stop_browser_sidecar, BrowserError, BrowserMode, BrowserStartResult,
    SidecarState, CHROME_DISCOVERY_PORT,
};
pub use terminal::{resize_terminal, spawn_terminal, stop_terminal, write_terminal, TerminalState};
pub use watcher::FileWatcherState;

use std::path::PathBuf;
use std::sync::Arc;

/// Configuration required to construct a [`TeshiEngine`].
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Absolute path to `browser_service.py`.
    pub browser_service_script: PathBuf,
    /// Absolute path to `winapp_service.py`.
    pub winapp_service_script: PathBuf,
    /// When true, embedded sidecar is started with `--no-preview-stream`
    /// (no JPEG frame loop). Intended for headless CI / replay only;
    /// desktop mode always keeps this `false`.
    pub embedded_no_preview_stream: bool,
}

/// Central holder for project, terminal, browser sidecar, and event bus state.
pub struct TeshiEngine {
    pub project: ProjectState,
    pub sidecar: SidecarState,
    pub terminal: TerminalState,
    pub watcher: FileWatcherState,
    pub locator_watcher: LocatorWatcherState,
    pub events: RuntimeEvents,
    pub browser_service_script: PathBuf,
    pub winapp_service_script: PathBuf,
    /// Prevents the embedded frame loop (--no-preview-stream).
    /// Read by `start_browser_sidecar` to decide whether to pass the flag.
    pub embedded_no_preview_stream: bool,
    /// Optional `teshi` CLI path injected into the embedded terminal as `TESHI_CLI`.
    embedded_terminal_teshi_cli: Option<PathBuf>,
}

impl TeshiEngine {
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
            winapp_service_script: config.winapp_service_script,
            embedded_no_preview_stream: config.embedded_no_preview_stream,
            embedded_terminal_teshi_cli: resolve_embedded_terminal_teshi_cli(),
        })
    }

    /// Returns the `teshi` CLI path injected into embedded terminal sessions, if any.
    pub fn embedded_terminal_teshi_cli(&self) -> Option<&std::path::Path> {
        self.embedded_terminal_teshi_cli.as_deref()
    }

    /// Emits initial `recent-loaded` if recent projects exist on disk.
    pub fn emit_initial_recent(&self) {
        if let Ok(recent) = load_recent_projects() {
            self.events.emit("recent-loaded", recent);
        }
    }
}

fn source_checkout_resource(name: &str) -> Option<PathBuf> {
    for root in [
        PathBuf::from("."),
        PathBuf::from(".."),
        PathBuf::from("../.."),
    ] {
        if root.join("Cargo.toml").is_file()
            && root.join("crates/teshi-engine/Cargo.toml").is_file()
        {
            let resource = root.join("resources").join(name);
            if resource.is_file() {
                return Some(resource);
            }
        }
    }
    None
}

/// Resolves `browser_service.py` from an override, a source checkout, or installed layouts.
pub fn default_browser_service_script() -> PathBuf {
    if let Ok(path) = std::env::var("TESHI_BROWSER_SERVICE") {
        return PathBuf::from(path);
    }

    // A debug executable can retain a stale target/debug/resources copy. When
    // launched inside this workspace, use the checked-out source of truth.
    if let Some(path) = source_checkout_resource("browser_service.py") {
        return path;
    }

    let mut candidates = vec![];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("share").join("browser_service.py"));
            candidates.push(exe_dir.join("..").join("share").join("browser_service.py"));
            candidates.push(exe_dir.join("resources").join("browser_service.py"));
        }
    }
    for path in &candidates {
        if path.is_file() {
            return path.clone();
        }
    }
    candidates
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("resources/browser_service.py"))
}

/// Resolves `winapp_service.py` from an override, a source checkout, or installed layouts.
pub fn default_winapp_service_script() -> PathBuf {
    if let Ok(path) = std::env::var("TESHI_WINAPP_SERVICE") {
        return PathBuf::from(path);
    }

    if let Some(path) = source_checkout_resource("winapp_service.py") {
        return path;
    }

    let mut candidates = vec![];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("share").join("winapp_service.py"));
            candidates.push(exe_dir.join("..").join("share").join("winapp_service.py"));
            candidates.push(exe_dir.join("resources").join("winapp_service.py"));
        }
    }
    for path in &candidates {
        if path.is_file() {
            return path.clone();
        }
    }
    candidates
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("resources/winapp_service.py"))
}

/// Resolves the `teshi` CLI binary for embedded terminal agents (`TESHI_CLI`).
///
/// Resolution order:
/// 1. Host `TESHI_CLI` when already set and non-empty
/// 2. Sibling `teshi` / `teshi.exe` next to the running host executable (dev `target/debug` or MSI `bin/`)
/// 3. The host executable itself when it is named `teshi` (e.g. `cargo run -- web`)
pub fn resolve_embedded_terminal_teshi_cli() -> Option<PathBuf> {
    if let Ok(existing) = std::env::var("TESHI_CLI") {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    let exe = std::env::current_exe().ok()?;
    embedded_teshi_cli_from_host_exe(&exe)
}

fn embedded_teshi_cli_from_host_exe(host_exe: &std::path::Path) -> Option<PathBuf> {
    if let Some(stem) = host_exe.file_stem().and_then(|s| s.to_str()) {
        if stem == "teshi" && host_exe.is_file() {
            return Some(dunce::simplified(host_exe).to_path_buf());
        }
    }

    let exe_dir = host_exe.parent()?;
    for name in embedded_teshi_cli_sibling_names() {
        let candidate = exe_dir.join(name);
        if candidate.is_file() {
            return Some(dunce::simplified(&candidate).to_path_buf());
        }
    }
    None
}

#[cfg(windows)]
fn embedded_teshi_cli_sibling_names() -> &'static [&'static str] {
    &["teshi.exe", "teshi"]
}

#[cfg(not(windows))]
fn embedded_teshi_cli_sibling_names() -> &'static [&'static str] {
    &["teshi"]
}

#[cfg(test)]
mod embedded_cli_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn embedded_cli_from_host_exe_uses_sibling_teshi() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let desktop = root.join("teshi-desktop.exe");
        let cli = root.join(embedded_teshi_cli_sibling_names()[0]);
        fs::write(&desktop, b"").unwrap();
        fs::write(&cli, b"").unwrap();

        let resolved = embedded_teshi_cli_from_host_exe(&desktop).unwrap();
        assert_eq!(resolved, dunce::simplified(&cli));
    }

    #[test]
    fn embedded_cli_from_host_exe_uses_teshi_web_host() {
        let dir = TempDir::new().unwrap();
        let cli = dir.path().join("teshi.exe");
        fs::write(&cli, b"").unwrap();

        let resolved = embedded_teshi_cli_from_host_exe(&cli).unwrap();
        assert_eq!(resolved, dunce::simplified(&cli));
    }

    #[test]
    fn embedded_cli_from_host_exe_missing_sibling_returns_none() {
        let dir = TempDir::new().unwrap();
        let desktop = dir.path().join("teshi-desktop.exe");
        fs::write(&desktop, b"").unwrap();

        assert!(embedded_teshi_cli_from_host_exe(&desktop).is_none());
    }
}
