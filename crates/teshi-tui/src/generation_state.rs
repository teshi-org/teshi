//! Persist and restore the TUI generation pipeline session under `.teshi/`.
//!
//! Restoring never grants test-point approval; review state comes only from
//! `testpoints/testpoints.json`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use teshi_agent::pipeline::{GenerationSessionState, GenerationStage, restore_stage_after_reload};
use teshi_core::authoring::TestPoint;

const GENERATION_STATE_FILE: &str = "generation-state.json";

/// Returns the path to `.teshi/generation-state.json` for a project root.
pub fn generation_state_path(project_root: &Path) -> PathBuf {
    project_root.join(".teshi").join(GENERATION_STATE_FILE)
}

/// Atomically-ish writes the generation session state (write then rename via temp).
///
/// # Errors
///
/// Returns an error when the directory cannot be created or the file cannot be written.
pub fn save_generation_state(project_root: &Path, state: &GenerationSessionState) -> Result<()> {
    let path = generation_state_path(project_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(state).context("serialize generation state")?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("rename into {}", path.display()))?;
    Ok(())
}

/// Loads generation session state when present.
///
/// # Errors
///
/// Returns an error when the file exists but cannot be read or parsed.
pub fn load_generation_state(project_root: &Path) -> Result<Option<GenerationSessionState>> {
    let path = generation_state_path(project_root);
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let state: GenerationSessionState =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(state))
}

/// Loads and reconciles generation stage with current test points without approving.
pub fn restore_generation_session(
    project_root: &Path,
    test_points: &[TestPoint],
) -> Result<(GenerationStage, Option<GenerationSessionState>)> {
    let Some(saved) = load_generation_state(project_root)? else {
        return Ok((GenerationStage::Idle, None));
    };
    let stage = restore_stage_after_reload(&saved, test_points);
    let mut restored = saved;
    restored.stage = stage;
    Ok((stage, Some(restored)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use teshi_agent::pipeline::{GenerationStage, Requirement};
    use teshi_core::authoring::{
        HierarchyPath, QuoteSelector, RequirementLink, ResolutionState, ReviewState, TestPoint,
        TextRange,
    };

    fn proposed_tp() -> TestPoint {
        TestPoint {
            id: "tp-1".into(),
            title: "Login".into(),
            objective: "User can log in".into(),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: HierarchyPath::new(vec!["Auth".into()]),
            review_state: ReviewState::Proposed,
            requirement_links: vec![RequirementLink {
                document_id: "doc-1".into(),
                document_revision: "rev".into(),
                position: TextRange::new(0, 4),
                quote: QuoteSelector {
                    quote: "text".into(),
                    prefix: String::new(),
                    suffix: String::new(),
                },
                resolution: ResolutionState::Resolved,
            }],
            scenario_refs: Vec::new(),
        }
    }

    #[test]
    fn save_and_restore_keeps_review_without_approving() {
        let dir = tempdir().unwrap();
        let state = GenerationSessionState {
            stage: GenerationStage::ReviewingTestPoints,
            requirement: Some(Requirement {
                feature_name: "Auth".into(),
                description: None,
                scenario_descriptions: vec!["login".into()],
                source_refs: vec![],
                tags: vec![],
            }),
            plan: None,
        };
        save_generation_state(dir.path(), &state).unwrap();
        let (stage, restored) = restore_generation_session(dir.path(), &[proposed_tp()]).unwrap();
        assert_eq!(stage, GenerationStage::ReviewingTestPoints);
        let restored = restored.expect("state");
        assert_eq!(restored.stage, GenerationStage::ReviewingTestPoints);
        assert_eq!(proposed_tp().review_state, ReviewState::Proposed);
    }

    #[test]
    fn missing_file_restores_idle() {
        let dir = tempdir().unwrap();
        let (stage, state) = restore_generation_session(dir.path(), &[]).unwrap();
        assert_eq!(stage, GenerationStage::Idle);
        assert!(state.is_none());
    }
}
