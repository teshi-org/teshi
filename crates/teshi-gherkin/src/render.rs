//! Structured Gherkin render payloads for teshi-desktop.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::gherkin::{
    BddFeature, BddScenario, BddStep, ExamplesTable, ScenarioKind, parse_feature,
};
use crate::gherkin_lang::GherkinLanguages;
use crate::highlight::{HighlightSpan, StepHighlightState, highlight_line_spans};

/// One step in a rendered scenario block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderStep {
    pub line_number: usize,
    pub keyword: String,
    pub text: String,
    pub keyword_kind: String,
}

/// One scenario or outline block in the structured view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderScenario {
    pub name: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub line_number: usize,
    pub steps: Vec<RenderStep>,
    pub examples: Vec<RenderExamples>,
}

/// Examples table attached to a scenario outline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderExamples {
    pub tags: Vec<String>,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub line_number: usize,
}

/// Top-level block in the structured panel (feature header, background, scenario).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RenderBlock {
    FeatureHeader {
        name: String,
        tags: Vec<String>,
        language: String,
    },
    Background {
        steps: Vec<RenderStep>,
    },
    Scenario(RenderScenario),
}

/// One highlighted source line for raw fallback view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderLine {
    pub line_number: usize,
    pub spans: Vec<HighlightSpan>,
}

/// Parse or highlight failure surfaced to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderError {
    pub message: String,
    pub line_number: Option<usize>,
}

/// Full payload for Panel1 Gherkin rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureRenderPayload {
    pub path: PathBuf,
    pub relative_path: String,
    pub structured: Vec<RenderBlock>,
    pub raw_lines: Vec<RenderLine>,
    pub error: Option<RenderError>,
}

/// Parses and renders a `.feature` file for the desktop Gherkin panel.
pub fn render_feature(
    content: &str,
    file_path: PathBuf,
    project_root: &Path,
) -> FeatureRenderPayload {
    let relative_path = path_relative_to_project(&file_path, project_root);
    let raw_lines = highlight_raw_lines(content, &detect_language(content));
    let feature = parse_feature(content, file_path.clone());
    let structured = build_structured_blocks(&feature);

    // The parser is lenient; add a minimal check: missing Feature header → parse failure,
    // so the UI can show an error bar and fall back to a raw text view.
    let error = detect_feature_error(content, &feature);

    FeatureRenderPayload {
        path: file_path,
        relative_path,
        // On parse failure, skip partial structured blocks so the frontend renders raw fallback.
        structured: if error.is_some() {
            Vec::new()
        } else {
            structured
        },
        raw_lines,
        error,
    }
}

/// Minimal validity check on the parse result; returns an error to surface in the UI.
fn detect_feature_error(content: &str, feature: &BddFeature) -> Option<RenderError> {
    if !feature.name.trim().is_empty() {
        return None;
    }
    // Locate the first non-empty, non-comment line as the error line for frontend highlighting.
    let line_number = content
        .lines()
        .position(|line| {
            let trimmed = line.trim_start();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .map(|i| i + 1);
    Some(RenderError {
        message: "Missing Feature header.".to_string(),
        line_number,
    })
}

fn detect_language(content: &str) -> String {
    for line in content.lines().take(10) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('#') {
            let meta = rest.trim();
            if let Some(code) = meta.strip_prefix("language:") {
                return code.trim().to_string();
            }
        }
    }
    "en".to_string()
}

fn highlight_raw_lines(content: &str, language_code: &str) -> Vec<RenderLine> {
    let lang = GherkinLanguages::global().get(language_code);
    let mut state = StepHighlightState::default();
    content
        .lines()
        .enumerate()
        .map(|(idx, line)| RenderLine {
            line_number: idx + 1,
            spans: highlight_line_spans(line, &mut state, lang),
        })
        .collect()
}

fn build_structured_blocks(feature: &BddFeature) -> Vec<RenderBlock> {
    let mut blocks = Vec::new();
    blocks.push(RenderBlock::FeatureHeader {
        name: feature.name.clone(),
        tags: feature.tags.clone(),
        language: feature.language.clone(),
    });
    if let Some(bg) = &feature.background {
        blocks.push(RenderBlock::Background {
            steps: bg.steps.iter().map(render_step).collect(),
        });
    }
    for scenario in &feature.scenarios {
        blocks.push(RenderBlock::Scenario(render_scenario(scenario)));
    }
    for rule in &feature.rules {
        blocks.push(RenderBlock::FeatureHeader {
            name: format!("Rule: {}", rule.name),
            tags: rule.tags.clone(),
            language: feature.language.clone(),
        });
        for scenario in &rule.scenarios {
            blocks.push(RenderBlock::Scenario(render_scenario(scenario)));
        }
    }
    blocks
}

fn render_scenario(scenario: &BddScenario) -> RenderScenario {
    RenderScenario {
        name: scenario.name.clone(),
        kind: match scenario.kind {
            ScenarioKind::Scenario => "scenario".to_string(),
            ScenarioKind::ScenarioOutline => "scenario_outline".to_string(),
        },
        tags: scenario.tags.clone(),
        line_number: scenario.line_number,
        steps: scenario.steps.iter().map(render_step).collect(),
        examples: scenario.examples.iter().map(render_examples).collect(),
    }
}

fn render_step(step: &BddStep) -> RenderStep {
    RenderStep {
        line_number: step.line_number,
        keyword: step.keyword.clone(),
        text: step.text.clone(),
        keyword_kind: format!("{:?}", step.keyword_type),
    }
}

fn render_examples(table: &ExamplesTable) -> RenderExamples {
    RenderExamples {
        tags: table.tags.clone(),
        headers: table.headers.clone(),
        rows: table.rows.clone(),
        line_number: table.line_number,
    }
}

fn path_relative_to_project(file_path: &Path, project_root: &Path) -> String {
    file_path
        .strip_prefix(project_root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| file_path.to_string_lossy().replace('\\', "/"))
}
