//! Locator workflow DTOs and normalisation helpers (pure data).
//! I/O, persistence, and watchers live in `teshi-engine`.

use chrono::Utc;
use serde::{Deserialize, Serialize};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_candidate: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_context_revision: Option<String>,
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
    #[serde(default = "legacy_step_binding_version")]
    pub format_version: u16,
    pub feature: String,
    pub steps: Vec<StepBinding>,
}

fn legacy_step_binding_version() -> u16 {
    1
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
    Confirmed,
    Rejected,
    Either,
}

/// Normalizes a Gherkin step text for stable binding lookup across line drift.
pub fn normalize_step_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Sanitizes a feature-relative path for use in filenames.
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

#[cfg(test)]
mod binding_version_tests {
    use super::*;

    #[test]
    fn reads_legacy_v1_binding_and_new_v2_reference_fields() {
        let legacy: StepBindingsFile = serde_json::from_str(
            r##"{"feature":"a.feature","steps":[{"step_line":1,"step_keyword":"Given","step_text":"x","step_text_normalized":"x","source":"binding","status":"confirmed","primary":{"strategy":"css","value":"#x","action":"click"}}]}"##,
        )
        .unwrap();
        assert_eq!(legacy.format_version, 1);
        assert!(legacy.steps[0].primary.element_reference.is_none());

        let current: StepBindingsFile = serde_json::from_str(
            r#"{"format_version":2,"feature":"a.feature","steps":[{"step_line":1,"step_keyword":"Given","step_text":"x","step_text_normalized":"x","source":"binding","status":"confirmed","primary":{"strategy":"reference","value":"@e1","action":"click","element_reference":"@e1","page_context_revision":"rev-a"}}]}"#,
        )
        .unwrap();
        assert_eq!(current.format_version, 2);
        assert_eq!(
            current.steps[0].primary.element_reference.as_deref(),
            Some("@e1")
        );
    }
}
