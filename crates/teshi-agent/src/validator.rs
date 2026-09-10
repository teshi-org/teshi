//! Compatibility adapter for the canonical `teshi-core` Gherkin validator.
//!
//! Agent callers historically consumed `ValidationIssue`. Keep that small
//! shape stable while delegating all rule ownership and source-aware checks to
//! `teshi-core`.

use std::path::Path;

use teshi_core::{
    BddProject, DiagnosticSeverity, ValidationDiagnostic, validate_feature_source,
    validate_project as validate_core_project,
};

/// Legacy severity shape retained for agent API compatibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueSeverity {
    Error,
    Warning,
    Suggestion,
}

/// Legacy agent validation issue, enriched with the canonical diagnostic data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub severity: IssueSeverity,
    pub file: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub code: String,
    pub message: String,
    pub suggestion: Option<String>,
}

impl From<ValidationDiagnostic> for ValidationIssue {
    fn from(diagnostic: ValidationDiagnostic) -> Self {
        Self {
            severity: match diagnostic.severity {
                DiagnosticSeverity::Error => IssueSeverity::Error,
                DiagnosticSeverity::Warning => IssueSeverity::Warning,
                DiagnosticSeverity::Suggestion => IssueSeverity::Suggestion,
            },
            file: diagnostic.path,
            line: Some(diagnostic.line),
            column: Some(diagnostic.column),
            code: diagnostic.code,
            message: diagnostic.message,
            suggestion: diagnostic.suggestion,
        }
    }
}

/// Validate a parsed project through the canonical core implementation.
pub fn validate_project(project: &BddProject) -> Vec<ValidationIssue> {
    validate_core_project(project)
        .diagnostics
        .into_iter()
        .map(ValidationIssue::from)
        .collect()
}

/// Validate raw Feature source through the canonical source-aware API.
pub fn validate_feature(content: &str, path: impl AsRef<Path>) -> Vec<ValidationIssue> {
    validate_feature_source(content, path)
        .diagnostics
        .into_iter()
        .map(ValidationIssue::from)
        .collect()
}

/// Format agent issues using the same severity grouping as the historical API.
pub fn format_validation_result(issues: &[ValidationIssue]) -> String {
    if issues.is_empty() {
        return "No validation issues found. Feature file(s) look good!".to_string();
    }

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut suggestions = Vec::new();

    for issue in issues {
        let location = match (issue.line, issue.column) {
            (Some(line), Some(column)) => format!("{}:{}:{}", issue.file, line, column),
            (Some(line), None) => format!("{}:{}", issue.file, line),
            _ => issue.file.clone(),
        };
        let mut entry = format!("  [{location}] [{}] {}", issue.code, issue.message);
        if let Some(suggestion) = &issue.suggestion {
            entry.push_str(&format!(" (suggestion: {suggestion})"));
        }
        match issue.severity {
            IssueSeverity::Error => errors.push(entry),
            IssueSeverity::Warning => warnings.push(entry),
            IssueSeverity::Suggestion => suggestions.push(entry),
        }
    }

    let mut out = format!(
        "Validation results — {} errors, {} warnings, {} suggestions\n",
        errors.len(),
        warnings.len(),
        suggestions.len()
    );
    append_group(&mut out, "Errors", &errors);
    append_group(&mut out, "Warnings", &warnings);
    append_group(&mut out, "Suggestions", &suggestions);
    out
}

fn append_group(output: &mut String, title: &str, entries: &[String]) {
    if entries.is_empty() {
        return;
    }
    output.push_str(&format!("\n## {title}\n"));
    for entry in entries {
        output.push_str(entry);
        output.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn adapter_exposes_core_missing_separator_diagnostic() {
        let issues = validate_feature(
            "# language: zh-CN\n功能: 登录\n  场景: 成功\n    当用户登录\n",
            "login.feature",
        );
        assert_eq!(issues.len(), 2);
        assert!(
            issues
                .iter()
                .any(|issue| { issue.code == "missing_step_separator" && issue.line == Some(4) })
        );
    }

    #[test]
    fn parsed_project_adapter_preserves_core_gwt_checks() {
        let project = BddProject {
            root_dir: PathBuf::from("/fake"),
            features: vec![teshi_core::BddFeature {
                file_path: PathBuf::from("login.feature"),
                name: "Login".into(),
                language: "en".into(),
                tags: vec![],
                description: vec![],
                background: None,
                rules: vec![],
                scenarios: vec![teshi_core::BddScenario {
                    name: "Missing Then".into(),
                    tags: vec![],
                    kind: teshi_core::ScenarioKind::Scenario,
                    steps: vec![
                        teshi_core::BddStep {
                            keyword: "Given".into(),
                            keyword_type: teshi_core::StepKeywordType::Given,
                            text: "the page is open".into(),
                            line_number: 3,
                            doc_string: None,
                            data_table: None,
                        },
                        teshi_core::BddStep {
                            keyword: "When".into(),
                            keyword_type: teshi_core::StepKeywordType::When,
                            text: "I act".into(),
                            line_number: 4,
                            doc_string: None,
                            data_table: None,
                        },
                    ],
                    examples: vec![],
                    line_number: 2,
                }],
                line_count: 4,
            }],
        };
        let issues = validate_project(&project);
        assert!(issues.iter().any(|issue| {
            issue.code == "missing_then" && issue.severity == IssueSeverity::Warning
        }));
    }

    #[test]
    fn formatted_result_includes_code_location_and_suggestion() {
        let issues = validate_feature(
            "# language: zh-CN\n功能: 登录\n  场景: 成功\n    当用户登录\n",
            "login.feature",
        );
        let formatted = format_validation_result(&issues);
        assert!(formatted.contains("missing_step_separator"));
        assert!(formatted.contains("login.feature:4:"));
        assert!(formatted.contains("suggestion"));
    }
}
