//! Gherkin render for the structured feature panel.
//! Also rebuilds the step reuse index after feature file changes.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use teshi_core::render_feature as render_feature_content;
use teshi_core::{FeatureRenderPayload, StepIndex};

use crate::events::RuntimeEvents;
use crate::TeshiEngine;

/// Debounce guard for step-index rebuild: skip if last rebuild was < 300 ms ago.
static LAST_REBUILD_MS: AtomicU64 = AtomicU64::new(0);

/// Renders a `.feature` file and starts watching it for changes.
pub fn render_feature(rt: &TeshiEngine, path: String) -> Result<FeatureRenderPayload, String> {
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    let file_path = canonical_child(Path::new(&path), &project_root)?;
    if file_path.extension().and_then(|s| s.to_str()) != Some("feature") {
        return Err("not a .feature file".into());
    }

    let content = fs::read_to_string(&file_path).map_err(|e| e.to_string())?;
    let payload = render_feature_content(&content, file_path, &project_root);

    rt.watcher
        .watch(&payload.path, &project_root, rt.events.clone())
        .map_err(|e| e.to_string())?;

    Ok(payload)
}

fn canonical_child(path: &Path, root: &Path) -> Result<PathBuf, String> {
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if !canonical.starts_with(&root) {
        return Err("path outside project root".into());
    }
    Ok(canonical)
}

/// Re-reads a feature file and emits `feature-refreshed`.
/// Also rebuilds the step reuse index and emits `step-index-updated`.
pub fn emit_feature_refresh(events: &RuntimeEvents, path: &Path, project_root: &Path) {
    if let Ok(content) = fs::read_to_string(path) {
        let payload = render_feature_content(&content, path.to_path_buf(), project_root);
        events.emit("feature-refreshed", payload);
    }
    // Also rebuild step index and emit event
    rebuild_and_emit_step_index(events, project_root);
}

/// Rebuilds the full StepIndex from all `.feature` files and emits a
/// `step-index-updated` event. Debounced to at most once per 300 ms.
pub fn rebuild_and_emit_step_index(events: &RuntimeEvents, project_root: &Path) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Debounce: skip if last rebuild was < 300 ms ago
    let last = LAST_REBUILD_MS.load(Ordering::Relaxed);
    if now < last + 300 {
        return;
    }
    LAST_REBUILD_MS.store(now, Ordering::Relaxed);

    let project = teshi_core::parse_project(project_root);
    let index = StepIndex::build(&project);

    let entries: Vec<serde_json::Value> = index
        .most_common(usize::MAX)
        .into_iter()
        .map(|(text, count)| {
            let locations = index.usages.get(&text).map(|locs| {
                locs.iter()
                    .map(|loc| {
                        let f = &project.features[loc.feature_idx];
                        let scenario = if loc.scenario_idx == usize::MAX {
                            "<Background>".to_string()
                        } else {
                            f.scenario_at(loc.scenario_idx)
                                .map(|s| s.name.clone())
                                .unwrap_or_else(|| format!("<unknown-{}>", loc.scenario_idx))
                        };
                        serde_json::json!({
                            "feature": f.file_path.strip_prefix(project_root).unwrap_or(&f.file_path).to_string_lossy(),
                            "scenario": scenario,
                            "line": loc.step_idx,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

            serde_json::json!({
                "text": text,
                "normalized": text,
                "count": count,
                "locations": locations,
            })
        })
        .collect();

    events.emit(
        "step-index-updated",
        serde_json::json!({
            "project_root": project_root.to_string_lossy(),
            "total_raw_steps": index.usages.values().map(|v| v.len()).sum::<usize>(),
            "unique_normalized": index.usages.len(),
            "num_features": project.features.len(),
            "entries": entries,
        }),
    );
}
