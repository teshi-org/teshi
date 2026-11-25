//! Context capsules for the AI Tab: last-focused item tracking and prompt assembly.
//!
//! Captures the user's most recently inspected item from `Explore`, `Issues`,
//! or `Test Points`, surfaces it as a pending suggestion pill above the AI
//! composer, and formats mounted capsules into the outgoing user prompt.

use teshi_core::authoring::{
    AuthoringArtifacts, RequirementDocumentContent, TestPoint, parse_markdown_frontmatter,
};
use teshi_core::gherkin::{BddFeature, BddScenario};

use crate::app::{CaseDetail, RunStatus};

/// Maximum characters included for a Markdown document body in an attached capsule.
const MAX_ISSUE_BODY_CHARS: usize = 6000;
/// Maximum characters included for a stack trace or log dump in an Explore capsule.
const MAX_ERROR_DETAIL_CHARS: usize = 3000;
/// Maximum number of log lines attached from an Explore Run Details record.
const MAX_LOG_LINES: usize = 30;

/// Origin tab of a focused or mounted AI context capsule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextSource {
    /// Selected document in the `Issues` tab.
    Issues,
    /// Selected test point in the `Test Points` tab.
    TestPoints,
    /// Selected scenario / Run Details in the `Explore` tab.
    Explore,
    /// `.feature` file selected via `@feature`.
    Feature,
}

impl ContextSource {
    /// Short bracket tag rendered inside capsule pills (`Issues`, `Test Points`, `Explore`, `Feature`).
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Issues => "Issues",
            Self::TestPoints => "Test Points",
            Self::Explore => "Explore",
            Self::Feature => "Feature",
        }
    }
}

/// One structured context item that can be suggested or attached above the AI composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCapsule {
    /// Stable deduplication key (e.g. `issue:doc-1`, `tp:tp-1`, `explore:a.feature:0`).
    pub id: String,
    /// Origin tab where the item was focused.
    pub source: ContextSource,
    /// Concise display title shown inside the pill.
    pub label: String,
    /// Full contextual payload assembled into the prompt on send.
    pub prompt_text: String,
}

impl ContextCapsule {
    /// Display string `[Source] Label` shown inside suggested and mounted capsules.
    pub fn badge_label(&self) -> String {
        format!("[{}] {}", self.source.tag(), self.label)
    }
}

/// Captures a context capsule from the currently selected Explore feature, scenario,
/// and optional Run Details (`CaseDetail`).
pub fn capsule_from_explore(
    feature: &BddFeature,
    relative_path: &str,
    scenario_idx: usize,
    scenario: Option<&BddScenario>,
    detail: Option<&CaseDetail>,
) -> ContextCapsule {
    let label = scenario
        .map(|s| s.name.trim())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            let name = feature.name.trim();
            if name.is_empty() { relative_path } else { name }
        })
        .to_string();

    let id = format!("explore:{relative_path}:{scenario_idx}");
    let mut lines = Vec::new();
    lines.push(format!(
        "Feature: {} ({relative_path})",
        feature.name.trim()
    ));
    if let Some(sc) = scenario {
        lines.push(format!("Scenario: {}", sc.name.trim()));
        if !sc.steps.is_empty() {
            lines.push("Steps:".into());
            for step in &sc.steps {
                lines.push(format!("  {} {}", step.keyword.trim(), step.text.trim()));
            }
        }
    }

    if let Some(detail) = detail {
        let status_str = match detail.status {
            RunStatus::Idle => "Idle",
            RunStatus::Running => "Running",
            RunStatus::Passed => "Passed",
            RunStatus::Failed => "Failed",
            RunStatus::Skipped => "Skipped",
            RunStatus::Blocked => "Blocked",
        };
        let mut header = format!("Run Status: {status_str}");
        if let Some(ms) = detail.duration_ms {
            header.push_str(&format!(" ({ms}ms)"));
        }
        lines.push(header);

        let failed_steps: Vec<_> = detail
            .steps
            .iter()
            .filter(|step| {
                step.status
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case("failed"))
                    || step
                        .message
                        .as_deref()
                        .is_some_and(|m| !m.trim().is_empty())
            })
            .collect();
        if !failed_steps.is_empty() {
            lines.push("Failed Steps:".into());
            for step in failed_steps {
                let mut row = format!("  - {}", step.text.trim());
                if let Some(msg) = step
                    .message
                    .as_deref()
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                {
                    row.push_str(&format!(": {msg}"));
                }
                lines.push(row);
            }
        }

        if let Some(msg) = detail
            .message
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            lines.push(format!(
                "Error Message:\n{}",
                truncate_chars(msg, MAX_ERROR_DETAIL_CHARS)
            ));
        }

        if let Some(stack) = detail
            .stack
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            lines.push(format!(
                "Stack Trace:\n{}",
                truncate_chars(stack, MAX_ERROR_DETAIL_CHARS)
            ));
        }

        if !detail.logs.is_empty() {
            let start = detail.logs.len().saturating_sub(MAX_LOG_LINES);
            let recent = detail.logs[start..].join("\n");
            lines.push(format!(
                "Logs:\n{}",
                truncate_chars(&recent, MAX_ERROR_DETAIL_CHARS)
            ));
        }

        let error_exchanges: Vec<_> = detail
            .exchanges
            .iter()
            .filter(|ex| {
                ex.status.is_some_and(|s| s >= 400) || ex.asserts.iter().any(|a| !a.passed)
            })
            .take(5)
            .collect();
        if !error_exchanges.is_empty() {
            lines.push("HTTP Errors:".into());
            for ex in error_exchanges {
                let status = ex
                    .status
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "ERR".into());
                let mut row = format!("  - {} {} -> {status}", ex.method, ex.url);
                let failed_asserts: Vec<&str> = ex
                    .asserts
                    .iter()
                    .filter(|a| !a.passed)
                    .map(|a| a.name.as_str())
                    .collect();
                if !failed_asserts.is_empty() {
                    row.push_str(&format!(" (failed asserts: {})", failed_asserts.join(", ")));
                }
                lines.push(row);
            }
        }
    }

    ContextCapsule {
        id,
        source: ContextSource::Explore,
        label,
        prompt_text: lines.join("\n"),
    }
}

/// Captures a context capsule from the currently selected Issue document.
pub fn capsule_from_issue(doc: &RequirementDocumentContent) -> ContextCapsule {
    let label = if doc.meta.title.trim().is_empty() {
        doc.meta.path.clone()
    } else {
        doc.meta.title.trim().to_string()
    };
    let parsed = parse_markdown_frontmatter(&doc.body);
    let mut lines = vec![
        format!("Issue ID: {}", doc.meta.id),
        format!("Title: {label}"),
        format!("Path: {}", doc.meta.path),
    ];
    if let Some(fm) = parsed.frontmatter {
        if let Some(kind) = fm.issue_type {
            lines.push(format!("Type: {kind}"));
        }
        if let Some(status) = fm.status {
            lines.push(format!("Status: {status}"));
        }
        if let Some(ext) = fm.external_ref {
            lines.push(format!("Ref: {ext}"));
        }
    }
    let body = parsed.body.trim();
    if !body.is_empty() {
        lines.push(String::new());
        lines.push(truncate_chars(body, MAX_ISSUE_BODY_CHARS));
    }

    ContextCapsule {
        id: format!("issue:{}", doc.meta.id),
        source: ContextSource::Issues,
        label,
        prompt_text: lines.join("\n"),
    }
}

/// Captures a context capsule from the currently selected Test Point.
pub fn capsule_from_test_point(
    tp: &TestPoint,
    artifacts: Option<&AuthoringArtifacts>,
) -> ContextCapsule {
    let label = if tp.title.trim().is_empty() {
        tp.id.clone()
    } else {
        tp.title.trim().to_string()
    };
    let mut lines = vec![
        format!("Test Point ID: {}", tp.id),
        format!("Title: {label}"),
        format!("Review State: {:?}", tp.review_state),
    ];
    if !tp.hierarchy_path.segments().is_empty() {
        lines.push(format!(
            "Hierarchy: {}",
            tp.hierarchy_path.segments().join(" / ")
        ));
    }
    if !tp.objective.trim().is_empty() {
        lines.push(format!("Objective: {}", tp.objective.trim()));
    }
    if let Some(pre) = tp
        .preconditions
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        lines.push(format!("Preconditions: {pre}"));
    }
    if let Some(exp) = tp
        .expected_outcomes
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        lines.push(format!("Expected Outcomes: {exp}"));
    }
    if !tp.requirement_links.is_empty() {
        lines.push("Linked Issues:".into());
        for link in &tp.requirement_links {
            let doc_title = artifacts
                .and_then(|a| a.index.documents.iter().find(|d| d.id == link.document_id))
                .map(|d| d.title.as_str())
                .unwrap_or(link.document_id.as_str());
            let quote = link.quote.quote.trim();
            if quote.is_empty() {
                lines.push(format!("  - {doc_title} ({})", link.document_id));
            } else {
                lines.push(format!("  - {doc_title}: \"{quote}\""));
            }
        }
    }
    if !tp.scenario_refs.is_empty() {
        lines.push("Linked Scenarios:".into());
        for sref in &tp.scenario_refs {
            if let Some(name) = sref
                .scenario_name
                .as_deref()
                .filter(|n| !n.trim().is_empty())
            {
                lines.push(format!("  - {} :: {name}", sref.feature_path));
            } else {
                lines.push(format!("  - {}", sref.feature_path));
            }
        }
    }

    ContextCapsule {
        id: format!("tp:{}", tp.id),
        source: ContextSource::TestPoints,
        label,
        prompt_text: lines.join("\n"),
    }
}

/// Captures a context capsule from an entire `.feature` file (used by `@feature`).
pub fn capsule_from_feature(feature: &BddFeature, relative_path: &str) -> ContextCapsule {
    let label = {
        let name = feature.name.trim();
        if name.is_empty() {
            relative_path.to_string()
        } else {
            name.to_string()
        }
    };
    let mut lines = vec![format!("Feature: {label} ({relative_path})")];
    if !feature.tags.is_empty() {
        lines.push(format!("Tags: {}", feature.tags.join(" ")));
    }
    let scenarios = feature.all_scenarios();
    if !scenarios.is_empty() {
        lines.push(format!("Scenarios ({}):", scenarios.len()));
        for scenario in scenarios.into_iter().take(40) {
            lines.push(format!("- Scenario: {}", scenario.name.trim()));
            for step in &scenario.steps {
                lines.push(format!("    {} {}", step.keyword.trim(), step.text.trim()));
            }
        }
    }
    let body = truncate_chars(&lines.join("\n"), MAX_ISSUE_BODY_CHARS);
    ContextCapsule {
        id: format!("feature:{relative_path}"),
        source: ContextSource::Feature,
        label,
        prompt_text: body,
    }
}

/// Prepends mounted context capsules to the outgoing user message.
///
/// Returns `user_msg` unchanged when `capsules` is empty.
pub fn assemble_prompt_with_capsules(user_msg: &str, capsules: &[ContextCapsule]) -> String {
    if capsules.is_empty() {
        return user_msg.to_string();
    }
    let mut sections = Vec::with_capacity(capsules.len());
    for capsule in capsules {
        sections.push(format!(
            "### {}\n{}",
            capsule.badge_label(),
            capsule.prompt_text.trim()
        ));
    }
    let context_block = sections.join("\n\n");
    let trimmed = user_msg.trim();
    if trimmed.is_empty() {
        format!("<attached_context>\n{context_block}\n</attached_context>")
    } else {
        format!("<attached_context>\n{context_block}\n</attached_context>\n\n{user_msg}")
    }
}

/// Splits an assembled prompt into its leading `<attached_context>...</attached_context>`
/// block and the trailing user-authored prompt text.
pub fn split_attached_context_block(message: &str) -> Option<(&str, &str)> {
    const START: &str = "<attached_context>\n";
    const END: &str = "\n</attached_context>";
    if !message.starts_with(START) {
        return None;
    }
    let end_rel = message.find(END)?;
    let prefix_end = end_rel + END.len();
    let prefix = &message[..prefix_end];
    let rest = &message[prefix_end..];
    if rest.is_empty() {
        Some((prefix, ""))
    } else {
        rest.strip_prefix("\n\n").map(|body| (prefix, body))
    }
}

/// Returns only the user-authored portion of `message`, stripping any leading
/// `<attached_context>...</attached_context>` block.
pub fn strip_attached_context_block(message: &str) -> &str {
    split_attached_context_block(message).map_or(message, |(_, body)| body)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let collected: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{collected}\n…(truncated)")
    } else {
        collected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use teshi_core::authoring::{DocumentRevision, RequirementDocumentMeta};

    #[test]
    fn explore_capsule_captures_failed_run_details() {
        let feature = teshi_core::gherkin::parse_feature(
            "Feature: Export\n  Scenario: 导出超时\n    When user exports CSV\n",
            PathBuf::from("features/export.feature"),
        );
        let detail = CaseDetail {
            case_id: "c1".into(),
            status: RunStatus::Failed,
            duration_ms: Some(30_012),
            message: Some("TimeoutError: export exceeded 30s".into()),
            stack: Some("at export.py:42".into()),
            attachments: Vec::new(),
            logs: vec!["starting export".into(), "timed out".into()],
            exchanges: Vec::new(),
            steps: vec![crate::app::StepRunRecord {
                step_id: "s1".into(),
                text: "user exports CSV".into(),
                status: Some("failed".into()),
                message: Some("timeout".into()),
            }],
            inspector_plaintext: false,
            started_at: None,
            screenshots: Vec::new(),
            historical: false,
        };

        let capsule = capsule_from_explore(
            &feature,
            "features/export.feature",
            0,
            feature.scenario_at(0),
            Some(&detail),
        );
        assert_eq!(capsule.source, ContextSource::Explore);
        assert_eq!(capsule.badge_label(), "[Explore] 导出超时");
        assert!(capsule.prompt_text.contains("Run Status: Failed (30012ms)"));
        assert!(
            capsule
                .prompt_text
                .contains("TimeoutError: export exceeded 30s")
        );
        assert!(capsule.prompt_text.contains("at export.py:42"));
        assert!(capsule.prompt_text.contains("timed out"));
    }

    #[test]
    fn issue_capsule_includes_frontmatter_and_body() {
        let doc = RequirementDocumentContent {
            meta: RequirementDocumentMeta::new(
                "doc-7",
                "export.md",
                "Export timeout",
                DocumentRevision::new("r1"),
            ),
            body: "---\ntype: bug\nstatus: testing\nref: GH-7\n---\n# Export timeout\n\nFix 30s timeout.\n"
                .into(),
        };
        let capsule = capsule_from_issue(&doc);
        assert_eq!(capsule.badge_label(), "[Issues] Export timeout");
        assert!(capsule.prompt_text.contains("Type: bug"));
        assert!(capsule.prompt_text.contains("Status: testing"));
        assert!(capsule.prompt_text.contains("Ref: GH-7"));
        assert!(capsule.prompt_text.contains("Fix 30s timeout."));
    }

    #[test]
    fn assemble_prompt_is_noop_when_empty_and_wraps_attached_capsules() {
        assert_eq!(assemble_prompt_with_capsules("hello", &[]), "hello");

        let capsule = ContextCapsule {
            id: "explore:a.feature:0".into(),
            source: ContextSource::Explore,
            label: "导出超时".into(),
            prompt_text: "Run Status: Failed\nError: 504".into(),
        };
        let assembled = assemble_prompt_with_capsules("分析报错", &[capsule]);
        assert!(assembled.starts_with("<attached_context>\n### [Explore] 导出超时\n"));
        assert!(assembled.contains("Error: 504"));
        assert!(assembled.ends_with("</attached_context>\n\n分析报错"));

        let (prefix, body) = split_attached_context_block(&assembled)
            .expect("leading attached_context block should split cleanly");
        assert!(prefix.starts_with("<attached_context>\n"));
        assert!(prefix.ends_with("</attached_context>"));
        assert_eq!(body, "分析报错");
        assert_eq!(strip_attached_context_block(&assembled), "分析报错");
    }

    #[test]
    fn split_attached_context_block_handles_empty_body_and_plain_prompts() {
        let capsule = ContextCapsule {
            id: "issue:doc-1".into(),
            source: ContextSource::Issues,
            label: "Login bug".into(),
            prompt_text: "Title: Login bug".into(),
        };
        let without_body = assemble_prompt_with_capsules("", &[capsule]);
        let (prefix, body) = split_attached_context_block(&without_body)
            .expect("empty-body attached_context block should split");
        assert_eq!(prefix, without_body);
        assert_eq!(body, "");
        assert_eq!(strip_attached_context_block(&without_body), "");

        assert_eq!(split_attached_context_block("plain user message"), None);
        assert_eq!(
            strip_attached_context_block("plain user message"),
            "plain user message"
        );
    }
}
