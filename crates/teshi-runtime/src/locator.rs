//! Locator workflow: active-step context, pending proposals, and per-feature MD persistence.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

    /// Starts watching the pending locator file under the opened project.
    pub fn watch_project(&self, project_root: &Path, rt: Arc<TeshiRuntime>) -> Result<()> {
        self.clear()?;
        let teshi = ensure_teshi_dir(project_root)?;
        let pending_path = pending_locator_path(project_root);

        let rt_watch = Arc::clone(&rt);
        let root = project_root.to_path_buf();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if event.paths.iter().any(|p| p == &pending_path) {
                        emit_pending_locator(&rt_watch, &root);
                    }
                }
            },
            Config::default().with_poll_interval(std::time::Duration::from_millis(300)),
        )?;
        watcher.watch(&teshi, RecursiveMode::NonRecursive)?;
        *self.watcher.lock().unwrap() = Some(watcher);
        emit_pending_locator(&rt, project_root);
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

fn locators_md_path(project_root: &Path, feature_relative_path: &str) -> PathBuf {
    let feature_path = project_root.join(feature_relative_path);
    let parent = feature_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| project_root.to_path_buf());
    let stem = feature_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "feature".to_string());
    parent.join(format!("{stem}.locators.md"))
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
            rt.events.emit("pending-locator-changed", Some(pending));
        }
        Err(err) => {
            tracing::warn!("failed to read pending locator: {err}");
            rt.events
                .emit("pending-locator-changed", Option::<PendingLocator>::None);
        }
    }
}

fn step_section_markers(step: &ActiveStep) -> (String, String) {
    let heading = format!(
        "### {} {} (L{})\n",
        step.step_keyword, step.step_text, step.step_line
    );
    let marker = format!("(L{})", step.step_line);
    (heading, marker)
}

fn render_step_section(step: &ActiveStep, candidate: &LocatorCandidate) -> String {
    let (heading, _) = step_section_markers(step);
    format!(
        "{heading}- **Primary** ({}): `{}`\n- **Action**: {}\n- **Confirmed**: {}\n- **Rationale**: {}\n\n",
        candidate.strategy,
        candidate.value,
        candidate.action,
        Utc::now().to_rfc3339(),
        candidate.rationale
    )
}

fn append_or_update_locators_md(
    project_root: &Path,
    step: &ActiveStep,
    candidate: &LocatorCandidate,
) -> Result<()> {
    let md_path = locators_md_path(project_root, &step.feature_relative_path);
    let file_header = format!("# Locators for {}\n\n", step.feature_relative_path);
    let scenario_header = format!(
        "## Scenario: {} (L{})\n\n",
        step.scenario_name, step.scenario_line
    );
    let step_section = render_step_section(step, candidate);
    let (_, step_marker) = step_section_markers(step);

    let mut content = if md_path.exists() {
        fs::read_to_string(&md_path).context("read locators md")?
    } else {
        file_header.clone()
    };

    if !content.starts_with("# Locators for") {
        content = format!("{file_header}{content}");
    }

    if let Some(idx) = content.find(&step_marker) {
        let start = content[..idx]
            .rfind("\n### ")
            .map(|pos| pos + 1)
            .unwrap_or_else(|| content[..idx].rfind("### ").unwrap_or(0));
        let tail = &content[start..];
        let end = tail[4..]
            .find("\n### ")
            .map(|offset| start + 4 + offset + 1)
            .unwrap_or(content.len());
        content.replace_range(start..end, step_section.trim_end());
        if !content.ends_with('\n') {
            content.push('\n');
        }
    } else {
        if !content.contains(&scenario_header) {
            content.push_str(&scenario_header);
        }
        content.push_str(&step_section);
    }

    if let Some(parent) = md_path.parent() {
        fs::create_dir_all(parent).context("create md parent directory")?;
    }
    fs::write(&md_path, content).context("write locators md")?;
    Ok(())
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

/// Confirms a pending locator and writes it to the per-feature markdown file.
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

    let path = pending_locator_path(&project_root);
    let mut pending: PendingLocator = read_json(&path).map_err(|e| e.to_string())?;
    if pending.status != "pending" {
        return Err("no pending locator proposal".into());
    }

    let mut candidate = pending
        .candidates
        .iter()
        .find(|c| c.rank == candidate_rank)
        .cloned()
        .ok_or_else(|| format!("candidate rank {candidate_rank} not found"))?;

    if let Some(value) = edited_value {
        candidate.value = value;
    }

    append_or_update_locators_md(&project_root, &pending.step_ref, &candidate)
        .map_err(|e| e.to_string())?;

    pending.status = "confirmed".to_string();
    write_json(&path, &pending).map_err(|e| e.to_string())?;
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

    let path = pending_locator_path(&project_root);
    if !path.exists() {
        return Err("no pending locator proposal".into());
    }
    let mut pending: PendingLocator = read_json(&path).map_err(|e| e.to_string())?;
    pending.status = "rejected".to_string();
    write_json(&path, &pending).map_err(|e| e.to_string())?;
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
}
