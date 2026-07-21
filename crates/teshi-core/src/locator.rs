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
