//! Locator workflow: active-step context, pending proposals, and step-binding persistence.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Utc;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use teshi_gherkin::parse_feature;

use crate::sidecar::{send_sidecar_command, SidecarState};
use crate::TeshiRuntime;

/// Selected Gherkin step context written for the Cursor agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveStep {
    pub feature_relative_path: String,
    pub scenario_line: usize,
    pub scenario_name: String,
    pub step_line: usize,
    pub step_keyword: String,
    pub step_text: String,
    #[serde(default = "default_updated_at")]
    pub updated_at: String,
}

fn default_updated_at() -> String {
    Utc::now().to_rfc3339()
}

/// One candidate locator proposed by the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocatorCandidate {
    pub rank: u32,
    pub strategy: String,
    pub value: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_arg: Option<String>,
    pub confidence: f64,
    pub rationale: String,
}

/// Highlight metadata attached to a pending proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HighlightInfo {
    pub candidate_rank: u32,
    pub applied: bool,
}

/// Agent-written locator proposal awaiting user confirmation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingLocator {
    pub step_ref: ActiveStep,
    pub candidates: Vec<LocatorCandidate>,
    pub highlight: Option<HighlightInfo>,
    pub status: String,
}

/// Persisted executable locator for one confirmed Gherkin step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocatorPrimary {
    pub strategy: String,
    pub value: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_arg: Option<String>,
}

/// One row in `.teshi/step-bindings/{feature}.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepBinding {
    pub step_line: usize,
    pub step_keyword: String,
    pub step_text: String,
    pub step_text_normalized: String,
    pub source: String,
    pub status: String,
    pub primary: LocatorPrimary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_at: Option<String>,
}

/// Per-feature binding index committed with the project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepBindingsFile {
    pub feature: String,
    pub steps: Vec<StepBinding>,
}

/// Status summary used by desktop/web step-tree badges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepBindingStatus {
    pub step_line: usize,
    pub step_text_normalized: String,
    pub status: String,
    pub source: String,
}

/// Result of waiting for a pending proposal to be confirmed or rejected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepWaitResult {
    pub status: String,
    pub reason: String,
}

/// Target terminal state for `teshi steps wait`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepWaitUntil {
    /// Return only when the proposal is confirmed.
    Confirmed,
    /// Return only when the proposal is rejected.
    Rejected,
    /// Return on either terminal state.
    Either,
}

/// Watches `.teshi/pending-locator.json` for agent proposals.
pub struct LocatorWatcherState {
    watcher: Mutex<Option<RecommendedWatcher>>,
}

impl Default for LocatorWatcherState {
    fn default() -> Self {
        Self::new()
    }
}

impl LocatorWatcherState {
    /// Creates an empty locator file watcher holder.
    pub fn new() -> Self {
        Self {
            watcher: Mutex::new(None),
        }
    }

    /// Starts watching `.teshi` locator files under the opened project.
    pub fn watch_project(&self, project_root: &Path, rt: Arc<TeshiRuntime>) -> Result<()> {
        self.clear()?;
        let teshi = ensure_teshi_dir(project_root)?;
        let pending_path = pending_locator_path(project_root);
        let active_path = active_step_path(project_root);

        let rt_watch = Arc::clone(&rt);
        let root = project_root.to_path_buf();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if event.paths.iter().any(|p| p == &pending_path) {
                        emit_pending_locator(&rt_watch, &root);
                    }
                    if event.paths.iter().any(|p| p == &active_path) {
                        emit_active_step_from_disk(&rt_watch, &root);
                    }
                }
            },
            Config::default().with_poll_interval(std::time::Duration::from_millis(300)),
        )?;
        watcher.watch(&teshi, RecursiveMode::NonRecursive)?;
        *self.watcher.lock().unwrap() = Some(watcher);
        emit_pending_locator(&rt, project_root);
        emit_active_step_from_disk(&rt, project_root);
        Ok(())
    }

    /// Stops watching the pending locator file.
    pub fn clear(&self) -> Result<()> {
        *self.watcher.lock().unwrap() = None;
        Ok(())
    }
}

fn teshi_dir(project_root: &Path) -> PathBuf {
    project_root.join(".teshi")
}

fn active_step_path(project_root: &Path) -> PathBuf {
    teshi_dir(project_root).join("active-step.json")
}

fn pending_locator_path(project_root: &Path) -> PathBuf {
    teshi_dir(project_root).join("pending-locator.json")
}

fn step_bindings_dir(project_root: &Path) -> PathBuf {
    teshi_dir(project_root).join("step-bindings")
}

pub fn sanitize_feature_path(feature_relative_path: &str) -> String {
    let mut out = String::with_capacity(feature_relative_path.len());
    for ch in feature_relative_path.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => out.push(ch),
            '/' | '\\' => out.push_str("__"),
            _ => out.push('_'),
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "feature".to_string()
    } else {
        trimmed.to_string()
    }
}

fn step_bindings_path(project_root: &Path, feature_relative_path: &str) -> PathBuf {
    step_bindings_dir(project_root).join(format!(
        "{}.json",
        sanitize_feature_path(feature_relative_path)
    ))
}

fn ensure_teshi_dir(project_root: &Path) -> Result<PathBuf> {
    let dir = teshi_dir(project_root);
    fs::create_dir_all(&dir).context("create .teshi directory")?;
    Ok(dir)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create parent directory")?;
    }
    let data = serde_json::to_string_pretty(value).context("serialize json")?;
    fs::write(path, data).context("write json file")?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let data = fs::read_to_string(path).context("read json file")?;
    serde_json::from_str(&data).context("parse json file")
}

/// Normalizes a Gherkin step text for stable binding lookup across line drift.
pub fn normalize_step_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn normalize_feature_relative_path(project_root: &Path, feature_path: &str) -> Result<String> {
    let path = Path::new(feature_path);
    if path.is_absolute() {
        let canonical_feature = path.canonicalize().context("canonicalize feature path")?;
        let root = project_root
            .canonicalize()
            .context("canonicalize project root")?;
        if !canonical_feature.starts_with(&root) {
            anyhow::bail!("feature path outside project root");
        }
        return Ok(canonical_feature
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| canonical_feature.to_string_lossy().replace('\\', "/")));
    }
    Ok(feature_path.replace('\\', "/"))
}

/// Resolves step metadata from a `.feature` file by line number.
pub fn resolve_step_context(
    project_root: &Path,
    feature_path: &Path,
    step_line: usize,
) -> Result<ActiveStep> {
    let canonical_feature = feature_path
        .canonicalize()
        .context("canonicalize feature path")?;
    let root = project_root
        .canonicalize()
        .context("canonicalize project root")?;
    if !canonical_feature.starts_with(&root) {
        anyhow::bail!("feature path outside project root");
    }

    let relative = canonical_feature
        .strip_prefix(&root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| canonical_feature.to_string_lossy().replace('\\', "/"));

    let content = fs::read_to_string(&canonical_feature).context("read feature file")?;
    let feature = parse_feature(&content, canonical_feature.clone());

    if let Some(bg) = &feature.background {
        if let Some(step) = bg.steps.iter().find(|s| s.line_number == step_line) {
            return Ok(ActiveStep {
                feature_relative_path: relative,
                scenario_line: bg.line_number,
                scenario_name: "Background".to_string(),
                step_line: step.line_number,
                step_keyword: step.keyword.clone(),
                step_text: step.text.clone(),
                updated_at: Utc::now().to_rfc3339(),
            });
        }
    }

    for scenario in feature
        .scenarios
        .iter()
        .chain(feature.rules.iter().flat_map(|r| r.scenarios.iter()))
    {
        if let Some(step) = scenario.steps.iter().find(|s| s.line_number == step_line) {
            return Ok(ActiveStep {
                feature_relative_path: relative,
                scenario_line: scenario.line_number,
                scenario_name: scenario.name.clone(),
                step_line: step.line_number,
                step_keyword: step.keyword.clone(),
                step_text: step.text.clone(),
                updated_at: Utc::now().to_rfc3339(),
            });
        }
    }

    anyhow::bail!("step line {step_line} not found in feature")
}

fn pending_is_blocking(project_root: &Path) -> Result<bool> {
    let path = pending_locator_path(project_root);
    if !path.exists() {
        return Ok(false);
    }
    let pending: PendingLocator = match read_json(&path) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    Ok(pending.status == "pending")
}

fn emit_pending_locator(rt: &TeshiRuntime, project_root: &Path) {
    let path = pending_locator_path(project_root);
    if !path.exists() {
        rt.events
            .emit("pending-locator-changed", Option::<PendingLocator>::None);
        return;
    }
    match read_json::<PendingLocator>(&path) {
        Ok(pending) => {
            if pending.status == "pending" {
                rt.events.emit("pending-locator-changed", Some(pending));
            } else {
                rt.events
                    .emit("pending-locator-changed", Option::<PendingLocator>::None);
            }
        }
        Err(err) => {
            tracing::warn!("failed to read pending locator: {err}");
            rt.events
                .emit("pending-locator-changed", Option::<PendingLocator>::None);
        }
    }
}

/// Emits `active-step-changed` when `.teshi/active-step.json` updates (CLI or Desktop).
fn emit_active_step_from_disk(rt: &TeshiRuntime, project_root: &Path) {
    let path = active_step_path(project_root);
    if !path.exists() {
        rt.events
            .emit("active-step-changed", Option::<ActiveStep>::None);
        return;
    }
    match read_json::<ActiveStep>(&path) {
        Ok(active) => {
            rt.events.emit("active-step-changed", Some(active));
        }
        Err(err) => {
            tracing::warn!("failed to read active step: {err}");
            rt.events
                .emit("active-step-changed", Option::<ActiveStep>::None);
        }
    }
}

/// Returns true when the on-disk active step disagrees with a pending proposal's step ref.
pub fn active_step_mismatch_with_pending(
    project_root: &Path,
    pending: &PendingLocator,
) -> Result<bool> {
    let path = active_step_path(project_root);
    if !path.exists() {
        return Ok(false);
    }
    let active: ActiveStep = read_json(&path)?;
    Ok(
        active.feature_relative_path != pending.step_ref.feature_relative_path
            || active.step_line != pending.step_ref.step_line,
    )
}

fn ensure_confirm_step_alignment(project_root: &Path, pending: &PendingLocator) -> Result<()> {
    if active_step_mismatch_with_pending(project_root, pending)? {
        anyhow::bail!(
            "active step (line {}) does not match pending proposal (line {}); \
             run `teshi steps select --line {}` or reject the proposal",
            read_active_step(project_root)
                .map(|s| s.step_line.to_string())
                .unwrap_or_else(|_| "?".to_string()),
            pending.step_ref.step_line,
            pending.step_ref.step_line
        );
    }
    Ok(())
}

fn read_step_bindings_file(
    project_root: &Path,
    feature_relative_path: &str,
) -> Result<StepBindingsFile> {
    let path = step_bindings_path(project_root, feature_relative_path);
    if !path.exists() {
        return Ok(StepBindingsFile {
            feature: feature_relative_path.to_string(),
            steps: Vec::new(),
        });
    }
    read_json(&path)
}

fn write_step_bindings_file(project_root: &Path, bindings: &StepBindingsFile) -> Result<()> {
    let path = step_bindings_path(project_root, &bindings.feature);
    let mut normalized = bindings.clone();
    normalized.steps.sort_by(|a, b| {
        a.step_line
            .cmp(&b.step_line)
            .then_with(|| a.step_text_normalized.cmp(&b.step_text_normalized))
    });
    write_json(&path, &normalized)
}

fn binding_from_candidate(step: &ActiveStep, candidate: &LocatorCandidate) -> StepBinding {
    StepBinding {
        step_line: step.step_line,
        step_keyword: step.step_keyword.clone(),
        step_text: step.step_text.clone(),
        step_text_normalized: normalize_step_text(&step.step_text),
        source: "binding".to_string(),
        status: "confirmed".to_string(),
        primary: LocatorPrimary {
            strategy: candidate.strategy.clone(),
            value: candidate.value.clone(),
            action: candidate.action.clone(),
            value_arg: candidate.value_arg.clone(),
        },
        confirmed_at: Some(Utc::now().to_rfc3339()),
    }
}

fn upsert_binding(
    project_root: &Path,
    step: &ActiveStep,
    candidate: &LocatorCandidate,
) -> Result<()> {
    let mut bindings = read_step_bindings_file(project_root, &step.feature_relative_path)?;
    let normalized_text = normalize_step_text(&step.step_text);
    let binding = binding_from_candidate(step, candidate);
    if let Some(existing) = bindings
        .steps
        .iter_mut()
        .find(|s| s.step_text_normalized == normalized_text)
    {
        *existing = binding;
    } else {
        bindings.steps.push(binding);
    }
    write_step_bindings_file(project_root, &bindings)
}

fn read_pending_locator(project_root: &Path) -> Result<Option<PendingLocator>> {
    let path = pending_locator_path(project_root);
    if !path.exists() {
        return Ok(None);
    }
    read_json(&path).map(Some)
}

fn write_pending_locator(project_root: &Path, pending: &PendingLocator) -> Result<()> {
    write_json(&pending_locator_path(project_root), pending)
}

fn confirm_pending_locator_file(
    project_root: &Path,
    candidate_rank: u32,
    edited_value: Option<String>,
) -> Result<PendingLocator> {
    let mut pending = read_pending_locator(project_root)?
        .ok_or_else(|| anyhow::anyhow!("no pending locator proposal"))?;
    if pending.status != "pending" {
        anyhow::bail!("no pending locator proposal");
    }

    ensure_confirm_step_alignment(project_root, &pending)?;

    let mut candidate = pending
        .candidates
        .iter()
        .find(|c| c.rank == candidate_rank)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("candidate rank {candidate_rank} not found"))?;

    if let Some(value) = edited_value {
        candidate.value = value;
    }

    upsert_binding(project_root, &pending.step_ref, &candidate)?;
    pending.status = "confirmed".to_string();
    write_pending_locator(project_root, &pending)?;
    Ok(pending)
}

fn reject_pending_locator_file(project_root: &Path) -> Result<PendingLocator> {
    let mut pending = read_pending_locator(project_root)?
        .ok_or_else(|| anyhow::anyhow!("no pending locator proposal"))?;
    pending.status = "rejected".to_string();
    write_pending_locator(project_root, &pending)?;
    Ok(pending)
}

async fn clear_browser_highlight(sidecar: &SidecarState) -> Result<(), String> {
    let ws_url = sidecar
        .browser_ws_url()
        .ok_or_else(|| "browser sidecar not running".to_string())?;
    send_sidecar_command(
        &ws_url,
        serde_json::json!({ "cmd": "clear_highlight", "request_id": "clear" }),
    )?;
    Ok(())
}

async fn highlight_browser_selector(sidecar: &SidecarState, selector: &str) -> Result<(), String> {
    let ws_url = sidecar
        .browser_ws_url()
        .ok_or_else(|| "browser sidecar not running".to_string())?;
    let response = send_sidecar_command(
        &ws_url,
        serde_json::json!({
            "cmd": "highlight_selector",
            "request_id": "highlight",
            "selector": selector,
        }),
    )?;
    if response.get("ok").and_then(|value| value.as_bool()) == Some(true) {
        return Ok(());
    }
    let message = response
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or("browser sidecar could not highlight selector");
    Err(message.to_string())
}

/// Highlights a selector candidate in the active browser session.
///
/// # Errors
///
/// Returns an error when no browser sidecar is running, the sidecar cannot be
/// reached, or the selector does not resolve to exactly one element.
pub async fn highlight_locator(rt: &TeshiRuntime, selector: String) -> Result<(), String> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err("selector is empty".to_string());
    }
    highlight_browser_selector(&rt.sidecar, selector).await
}

/// Writes the active step context file for the Cursor agent.
pub async fn sync_active_step(
    rt: &TeshiRuntime,
    feature_path: String,
    step_line: u32,
) -> Result<ActiveStep, String> {
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    if pending_is_blocking(&project_root).map_err(|e| e.to_string())? {
        return Err(
            "A locator proposal is pending confirmation. Accept or reject it before selecting another step."
                .into(),
        );
    }

    let active = resolve_step_context(&project_root, Path::new(&feature_path), step_line as usize)
        .map_err(|e| e.to_string())?;

    ensure_teshi_dir(&project_root).map_err(|e| e.to_string())?;
    write_json(&active_step_path(&project_root), &active).map_err(|e| e.to_string())?;
    rt.events.emit("active-step-changed", active.clone());
    Ok(active)
}

/// Returns the current active step context, if any.
pub fn get_active_step(rt: &TeshiRuntime) -> Result<Option<ActiveStep>, String> {
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;
    let path = active_step_path(&project_root);
    if !path.exists() {
        return Ok(None);
    }
    read_json(&path).map_err(|e| e.to_string()).map(Some)
}

/// Returns the pending locator proposal, if any.
pub fn get_pending_locator(rt: &TeshiRuntime) -> Result<Option<PendingLocator>, String> {
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;
    let path = pending_locator_path(&project_root);
    if !path.exists() {
        return Ok(None);
    }
    let pending: PendingLocator = read_json(&path).map_err(|e| e.to_string())?;
    if pending.status != "pending" {
        return Ok(None);
    }
    Ok(Some(pending))
}

/// Reads the selected active step directly from a project checkout.
///
/// # Errors
///
/// Returns an error when `.teshi/active-step.json` is missing or malformed.
pub fn read_active_step(project_root: &Path) -> Result<ActiveStep> {
    read_json(&active_step_path(project_root))
}

/// Reads the current pending locator proposal directly from a project checkout.
///
/// # Errors
///
/// Returns an error when `.teshi/pending-locator.json` is malformed.
pub fn read_pending(project_root: &Path) -> Result<Option<PendingLocator>> {
    read_pending_locator(project_root)
}

/// Writes an agent proposal to `.teshi/pending-locator.json`.
///
/// # Errors
///
/// Returns an error when the proposal cannot be serialized or written.
pub fn propose_locator(project_root: &Path, pending: PendingLocator) -> Result<()> {
    ensure_teshi_dir(project_root)?;
    write_pending_locator(project_root, &pending)
}

/// Confirms a pending proposal and upserts the selected candidate into step-bindings.
///
/// # Errors
///
/// Returns an error when there is no pending proposal, the rank is unknown,
/// or `.teshi/step-bindings` cannot be written.
pub fn confirm_pending_locator(
    project_root: &Path,
    candidate_rank: u32,
    edited_value: Option<String>,
) -> Result<PendingLocator> {
    confirm_pending_locator_file(project_root, candidate_rank, edited_value)
}

/// Rejects a pending proposal without writing a binding.
///
/// # Errors
///
/// Returns an error when there is no pending proposal or the status file cannot be written.
pub fn reject_pending_locator(project_root: &Path) -> Result<PendingLocator> {
    reject_pending_locator_file(project_root)
}

/// Returns all persisted step bindings for a feature.
///
/// # Errors
///
/// Returns an error when an existing binding file cannot be parsed.
pub fn list_step_bindings(
    project_root: &Path,
    feature_relative_path: &str,
) -> Result<StepBindingsFile> {
    read_step_bindings_file(project_root, feature_relative_path)
}

/// Removes a confirmed binding for one step line.
///
/// # Errors
///
/// Returns an error when no binding exists for the line or the file cannot be written.
pub fn unbind_step_binding(
    project_root: &Path,
    feature_relative_path: &str,
    step_line: usize,
) -> Result<Option<StepBinding>> {
    let feature_relative = normalize_feature_relative_path(project_root, feature_relative_path)?;
    let mut bindings = read_step_bindings_file(project_root, &feature_relative)?;
    let Some(idx) = bindings.steps.iter().position(|s| s.step_line == step_line) else {
        return Ok(None);
    };
    let removed = bindings.steps.remove(idx);
    write_step_bindings_file(project_root, &bindings)?;
    Ok(Some(removed))
}

/// Resolves confirmed binding steps up to an optional line number.
///
/// # Errors
///
/// Returns an error when the binding file cannot be parsed.
pub fn resolve_step_bindings(
    project_root: &Path,
    feature_relative_path: &str,
    until_line: Option<usize>,
) -> Result<Vec<StepBinding>> {
    let bindings = read_step_bindings_file(project_root, feature_relative_path)?;
    Ok(bindings
        .steps
        .into_iter()
        .filter(|step| step.status == "confirmed")
        .filter(|step| until_line.is_none_or(|line| step.step_line <= line))
        .collect())
}

/// One executable step line in a feature file with binding status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureStepRef {
    pub step_line: usize,
    pub step_keyword: String,
    pub step_text: String,
    pub scenario_name: String,
    pub status: String,
}

/// Writes `.teshi/active-step.json` for CLI and Desktop agents without a full runtime handle.
///
/// # Errors
///
/// Returns an error when a proposal is pending, the step line is invalid, or the file cannot be written.
pub fn write_active_step(
    project_root: &Path,
    feature_path: &str,
    step_line: usize,
) -> Result<ActiveStep> {
    if pending_is_blocking(project_root)? {
        anyhow::bail!(
            "A locator proposal is pending confirmation. Accept or reject it before selecting another step."
        );
    }
    let feature_relative = normalize_feature_relative_path(project_root, feature_path)?;
    let feature_abs = project_root.join(&feature_relative);
    let active = resolve_step_context(project_root, &feature_abs, step_line)?;
    ensure_teshi_dir(project_root)?;
    write_json(&active_step_path(project_root), &active)?;
    Ok(active)
}

fn confirmed_binding_lines(project_root: &Path, feature_relative: &str) -> Result<BTreeSet<usize>> {
    let bindings = read_step_bindings_file(project_root, feature_relative)?;
    Ok(bindings
        .steps
        .into_iter()
        .filter(|s| s.status == "confirmed")
        .map(|s| s.step_line)
        .collect())
}

fn collect_feature_steps(
    content: &str,
    feature_abs: PathBuf,
) -> Result<Vec<(usize, String, String, String)>> {
    let feature = parse_feature(content, feature_abs);
    let mut out = Vec::new();
    if let Some(bg) = &feature.background {
        for step in &bg.steps {
            out.push((
                step.line_number,
                step.keyword.clone(),
                step.text.clone(),
                "Background".to_string(),
            ));
        }
    }
    for scenario in feature
        .scenarios
        .iter()
        .chain(feature.rules.iter().flat_map(|r| r.scenarios.iter()))
    {
        for step in &scenario.steps {
            out.push((
                step.line_number,
                step.keyword.clone(),
                step.text.clone(),
                scenario.name.clone(),
            ));
        }
    }
    out.sort_by_key(|(line, _, _, _)| *line);
    Ok(out)
}

/// Lists every step line in a feature with binding status (`confirmed`, `pending`, `unbound`).
///
/// # Errors
///
/// Returns an error when the feature cannot be read or parsed.
pub fn list_feature_step_refs(
    project_root: &Path,
    feature_path: &str,
) -> Result<Vec<FeatureStepRef>> {
    let feature_relative = normalize_feature_relative_path(project_root, feature_path)?;
    let feature_abs = project_root.join(&feature_relative);
    let content = fs::read_to_string(&feature_abs).context("read feature file")?;
    let confirmed = confirmed_binding_lines(project_root, &feature_relative)?;
    let pending_line = read_pending_locator(project_root)?
        .filter(|p| p.status == "pending" && p.step_ref.feature_relative_path == feature_relative)
        .map(|p| p.step_ref.step_line);
    let steps = collect_feature_steps(&content, feature_abs)?;
    Ok(steps
        .into_iter()
        .map(|(step_line, step_keyword, step_text, scenario_name)| {
            let status = if confirmed.contains(&step_line) {
                "confirmed".to_string()
            } else if pending_line == Some(step_line) {
                "pending".to_string()
            } else {
                "unbound".to_string()
            };
            FeatureStepRef {
                step_line,
                step_keyword,
                step_text,
                scenario_name,
                status,
            }
        })
        .collect())
}

/// Returns the first unbound step in source order, if any.
pub fn first_unbound_feature_step(
    project_root: &Path,
    feature_path: &str,
) -> Result<Option<FeatureStepRef>> {
    Ok(list_feature_step_refs(project_root, feature_path)?
        .into_iter()
        .find(|s| s.status == "unbound"))
}

/// Returns per-step binding statuses for desktop/web badges.
///
/// Pending proposals override persisted status for their target step.
///
/// # Errors
///
/// Returns an error when binding or pending files cannot be parsed.
pub fn step_binding_statuses(
    project_root: &Path,
    feature_relative_path: &str,
) -> Result<Vec<StepBindingStatus>> {
    let feature_relative_path =
        normalize_feature_relative_path(project_root, feature_relative_path)?;
    let mut statuses = BTreeMap::<String, StepBindingStatus>::new();
    for step in read_step_bindings_file(project_root, &feature_relative_path)?.steps {
        statuses.insert(
            step.step_text_normalized.clone(),
            StepBindingStatus {
                step_line: step.step_line,
                step_text_normalized: step.step_text_normalized,
                status: step.status,
                source: step.source,
            },
        );
    }
    if let Some(pending) = read_pending_locator(project_root)? {
        if pending.step_ref.feature_relative_path == feature_relative_path
            && pending.status == "pending"
        {
            let normalized = normalize_step_text(&pending.step_ref.step_text);
            statuses.insert(
                normalized.clone(),
                StepBindingStatus {
                    step_line: pending.step_ref.step_line,
                    step_text_normalized: normalized,
                    status: "pending".to_string(),
                    source: "pending".to_string(),
                },
            );
        }
    }
    Ok(statuses.into_values().collect())
}

/// Waits until the pending proposal reaches the requested terminal state.
///
/// When `auto_confirm` is true and the deadline passes with a still-pending proposal,
/// confirms automatically unless the active step disagrees with the proposal (then rejects).
///
/// # Errors
///
/// Returns an error when the wait times out without auto-confirm, or files cannot be read.
pub fn wait_for_step_status(
    project_root: &Path,
    until: StepWaitUntil,
    timeout: Duration,
    auto_confirm: bool,
) -> Result<StepWaitResult> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(pending) = read_pending_locator(project_root)? {
            let terminal = match pending.status.as_str() {
                "confirmed" => Some(StepWaitUntil::Confirmed),
                "rejected" => Some(StepWaitUntil::Rejected),
                _ => None,
            };
            if let Some(state) = terminal {
                if until == StepWaitUntil::Either
                    || until == state
                    || state == StepWaitUntil::Rejected
                {
                    let status = pending.status;
                    return Ok(StepWaitResult {
                        reason: status.clone(),
                        status,
                    });
                }
            }
        }
        if Instant::now() >= deadline {
            if !auto_confirm {
                anyhow::bail!("timed out waiting for step proposal");
            }
            let Some(pending) = read_pending_locator(project_root)? else {
                anyhow::bail!("timed out waiting for step proposal");
            };
            if pending.status != "pending" {
                return Ok(StepWaitResult {
                    status: pending.status.clone(),
                    reason: pending.status,
                });
            }
            if active_step_mismatch_with_pending(project_root, &pending)? {
                reject_pending_locator_file(project_root)?;
                return Ok(StepWaitResult {
                    status: "rejected".to_string(),
                    reason: "auto-confirm skipped: active step mismatch".to_string(),
                });
            }
            confirm_pending_locator_file(project_root, 1, None)?;
            return Ok(StepWaitResult {
                status: "confirmed".to_string(),
                reason: "auto-confirmed after timeout".to_string(),
            });
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Confirms a pending locator and writes it to the per-feature step-binding file.
pub async fn confirm_locator(
    rt: &TeshiRuntime,
    candidate_rank: u32,
    edited_value: Option<String>,
) -> Result<(), String> {
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    confirm_pending_locator_file(&project_root, candidate_rank, edited_value)
        .map_err(|e| e.to_string())?;
    clear_browser_highlight(&rt.sidecar)
        .await
        .map_err(|e| e.to_string())?;
    emit_pending_locator(rt, &project_root);
    Ok(())
}

/// Rejects a pending locator proposal and clears the browser highlight.
pub async fn reject_locator(rt: &TeshiRuntime) -> Result<(), String> {
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    reject_pending_locator_file(&project_root).map_err(|e| e.to_string())?;
    clear_browser_highlight(&rt.sidecar)
        .await
        .map_err(|e| e.to_string())?;
    emit_pending_locator(rt, &project_root);
    Ok(())
}

/// Clears a stale pending proposal without persisting a locator.
pub async fn abandon_pending_locator(rt: &TeshiRuntime) -> Result<(), String> {
    reject_locator(rt).await
}

/// Removes a confirmed step binding for the open project.
pub async fn unbind_step(
    rt: &TeshiRuntime,
    feature_path: String,
    step_line: u32,
) -> Result<Option<StepBinding>, String> {
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;
    unbind_step_binding(&project_root, &feature_path, step_line as usize).map_err(|e| e.to_string())
}

/// Starts watching pending locator changes for the opened project.
pub fn start_locator_watch(
    watcher: &LocatorWatcherState,
    project_root: &Path,
    rt: Arc<TeshiRuntime>,
) -> Result<(), String> {
    watcher
        .watch_project(project_root, rt)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn resolve_step_context_finds_scenario_step() {
        let dir = TempDir::new().unwrap();
        let feature_path = dir.path().join("login.feature");
        let mut file = fs::File::create(&feature_path).unwrap();
        writeln!(
            file,
            "Feature: Login\n\n  Scenario: User logs in\n    When I click the login button"
        )
        .unwrap();

        let active = resolve_step_context(dir.path(), &feature_path, 4).unwrap();
        assert_eq!(active.scenario_name, "User logs in");
        assert_eq!(active.step_keyword, "When");
        assert_eq!(active.step_text, "I click the login button");
    }

    #[test]
    fn step_binding_statuses_accepts_absolute_feature_path() {
        let dir = TempDir::new().unwrap();
        let feature_path = dir.path().join("features").join("login.feature");
        fs::create_dir_all(feature_path.parent().unwrap()).unwrap();
        let mut file = fs::File::create(&feature_path).unwrap();
        writeln!(
            file,
            "Feature: Login\n\n  Scenario: User logs in\n    When I click the login button"
        )
        .unwrap();

        let active = resolve_step_context(dir.path(), &feature_path, 4).unwrap();
        let candidate = LocatorCandidate {
            rank: 1,
            strategy: "css".to_string(),
            value: "#login".to_string(),
            action: "click".to_string(),
            value_arg: None,
            confidence: 0.9,
            rationale: "unique id".to_string(),
        };
        upsert_binding(dir.path(), &active, &candidate).unwrap();

        let statuses = step_binding_statuses(dir.path(), feature_path.to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].status, "confirmed");
        assert_eq!(statuses[0].step_line, 4);
    }

    #[test]
    fn upsert_binding_persists_uia_strategy() {
        let dir = TempDir::new().unwrap();
        let feature_path = dir.path().join("native.feature");
        let mut file = fs::File::create(&feature_path).unwrap();
        writeln!(
            file,
            "Feature: Native\n\n  Scenario: User clicks login\n    When I click the login button"
        )
        .unwrap();

        let active = resolve_step_context(dir.path(), &feature_path, 4).unwrap();
        let candidate = LocatorCandidate {
            rank: 1,
            strategy: "uia".to_string(),
            value: "uia:automation_id=LoginButton".to_string(),
            action: "click".to_string(),
            value_arg: None,
            confidence: 0.9,
            rationale: "unique AutomationId".to_string(),
        };
        upsert_binding(dir.path(), &active, &candidate).unwrap();

        let bindings = list_step_bindings(dir.path(), "native.feature").unwrap();
        assert_eq!(bindings.steps.len(), 1);
        assert_eq!(bindings.steps[0].primary.strategy, "uia");
        assert_eq!(
            bindings.steps[0].primary.value,
            "uia:automation_id=LoginButton"
        );
    }
}
