//! Gherkin validation module.
//!
//! Provides functions to validate parsed feature files for common issues such
//! as missing Given/When/Then ordering, missing Examples tables, duplicate
//! scenario names, and overly long scenarios.

use std::collections::HashMap;

use crate::agent::skills::SkillRegistry;
use crate::gherkin::{BddProject, BddScenario, ScenarioKind};

/// The severity of a validation issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueSeverity {
    Error,
    Warning,
    Suggestion,
}

/// A single validation issue found during feature file analysis.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub severity: IssueSeverity,
    pub file: String,
    pub line: Option<usize>,
    pub message: String,
}

/// Validate a complete parsed project for common Gherkin issues.
///
/// Checks performed:
/// - Scenarios should start with "Given"
/// - Scenarios should have a "When" and "Then" step
/// - Too many steps in a single scenario
/// - Scenario Outlines should have Examples tables
/// - Examples tables should have headers
/// - Duplicate scenario names
pub fn validate_project(project: &BddProject) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    for feature in &project.features {
        let path = feature.file_path.to_string_lossy();

        for sc in &feature.scenarios {
            // Check Given/When/Then order
            let keywords: Vec<&str> = sc.steps.iter().map(|s| s.keyword.trim()).collect();

            if !keywords.is_empty() {
                let first = keywords[0];
                if first != "Given" {
                    issues.push(ValidationIssue {
                        severity: IssueSeverity::Warning,
                        file: path.to_string(),
                        line: Some(sc.line_number),
                        message: format!(
                            "Scenario '{}' should start with 'Given' (starts with '{}')",
                            sc.name, first
                        ),
                    });
                }

                let has_when = keywords.contains(&"When");
                let has_then = keywords.contains(&"Then");

                // Check incomplete Given-When-Then chain
                if sc.steps.len() >= 2 && !has_when {
                    issues.push(ValidationIssue {
                        severity: IssueSeverity::Warning,
                        file: path.to_string(),
                        line: Some(sc.line_number),
                        message: format!(
                            "Scenario '{}' has {} steps but no 'When' step",
                            sc.name,
                            sc.steps.len()
                        ),
                    });
                }

                if sc.steps.len() >= 2 && !has_then {
                    issues.push(ValidationIssue {
                        severity: IssueSeverity::Suggestion,
                        file: path.to_string(),
                        line: Some(sc.line_number),
                        message: format!(
                            "Scenario '{}' has {} steps but no 'Then' step",
                            sc.name,
                            sc.steps.len()
                        ),
                    });
                }
            }

            // Check for too many steps
            if sc.steps.len() > 10 {
                issues.push(ValidationIssue {
                    severity: IssueSeverity::Suggestion,
                    file: path.to_string(),
                    line: Some(sc.line_number),
                    message: format!(
                        "Scenario '{}' has {} steps (consider splitting)",
                        sc.name,
                        sc.steps.len()
                    ),
                });
            }

            // Check Scenario Outline has Examples
            if matches!(sc.kind, ScenarioKind::ScenarioOutline) && sc.examples.is_empty() {
                issues.push(ValidationIssue {
                    severity: IssueSeverity::Error,
                    file: path.to_string(),
                    line: Some(sc.line_number),
                    message: format!("Scenario Outline '{}' has no Examples table", sc.name),
                });
            }

            // Check Examples placeholders match headers
            if !sc.examples.is_empty() {
                for (ei, ex) in sc.examples.iter().enumerate() {
                    if ex.headers.is_empty() {
                        issues.push(ValidationIssue {
                            severity: IssueSeverity::Error,
                            file: path.to_string(),
                            line: Some(ex.line_number),
                            message: format!(
                                "Examples table {} in '{}' has no headers",
                                ei, sc.name
                            ),
                        });
                    }
                }
            }
        }

        // Check duplicate scenario names
        let mut names = HashMap::new();
        for sc in &feature.scenarios {
            let entry = names.entry(sc.name.clone()).or_insert(0usize);
            *entry += 1;
        }
        for (name, count) in names {
            if count > 1 {
                issues.push(ValidationIssue {
                    severity: IssueSeverity::Error,
                    file: path.to_string(),
                    line: None,
                    message: format!(
                        "Duplicate scenario name '{}' (appears {} times)",
                        name, count
                    ),
                });
            }
        }
    }

    issues
}

/// Format validation issues into a human-readable string.
pub fn format_validation_result(issues: &[ValidationIssue]) -> String {
    if issues.is_empty() {
        return "No validation issues found. Feature file(s) look good!".to_string();
    }

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut suggestions = Vec::new();

    for issue in issues {
        let location = match &issue.line {
            Some(line) => format!("{}:{}", issue.file, line),
            None => issue.file.clone(),
        };
        let entry = format!("  [{location}] {}", issue.message);
        match issue.severity {
            IssueSeverity::Error => errors.push(entry),
            IssueSeverity::Warning => warnings.push(entry),
            IssueSeverity::Suggestion => suggestions.push(entry),
        }
    }

    let mut out = String::new();
    out.push_str(&format!(
        "Validation results — {} errors, {} warnings, {} suggestions\n",
        errors.len(),
        warnings.len(),
        suggestions.len()
    ));

    if !errors.is_empty() {
        out.push_str("\n## Errors\n");
        for e in &errors {
            out.push_str(e);
            out.push('\n');
        }
    }

    if !warnings.is_empty() {
        out.push_str("\n## Warnings\n");
        for w in &warnings {
            out.push_str(w);
            out.push('\n');
        }
    }

    if !suggestions.is_empty() {
        out.push_str("\n## Suggestions\n");
        for s in &suggestions {
            out.push_str(s);
            out.push('\n');
        }
    }

    out
}

/// Check if a feature's scenarios cover the patterns recommended by matching
/// skill templates. Returns suggestions for missing scenario types.
pub fn check_coverage(
    feature_name: &str,
    scenarios: &[BddScenario],
    skill_registry: &SkillRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    if skill_registry.is_empty() {
        return issues;
    }

    // Match skills against the feature name
    let matched = skill_registry.match_skills(feature_name);
    if matched.is_empty() {
        return issues;
    }

    // Extract scenario names from the generated feature
    let generated_names: Vec<&str> = scenarios.iter().map(|s| s.name.as_str()).collect();
    let generated_lower: Vec<String> = generated_names.iter().map(|n| n.to_lowercase()).collect();

    for skill in &matched {
        // Parse "Recommended Scenarios" section from skill content
        let recommendations = extract_recommended_scenarios(&skill.content);
        if recommendations.is_empty() {
            continue;
        }

        let mut covered: usize = 0;
        for rec in &recommendations {
            let rec_lower = rec.to_lowercase();
            let found = generated_lower.iter().any(|n| {
                // Check if any generated scenario name contains keywords from the recommendation
                let rec_keywords: Vec<&str> = rec_lower
                    .split_whitespace()
                    .filter(|w| w.len() > 3) // only meaningful words
                    .collect();
                let match_count = rec_keywords.iter().filter(|kw| n.contains(*kw)).count();
                // Match if at least half of the keywords are found
                !rec_keywords.is_empty() && match_count >= rec_keywords.len().max(1) / 2
            });

            if !found {
                issues.push(ValidationIssue {
                    severity: IssueSeverity::Suggestion,
                    file: String::new(),
                    line: None,
                    message: format!(
                        "Consider adding a scenario for '{}' (recommended by template '{}')",
                        rec, skill.name
                    ),
                });
            } else {
                covered += 1;
            }
        }

        if covered < recommendations.len() && recommendations.len() > 1 {
            issues.push(ValidationIssue {
                severity: IssueSeverity::Suggestion,
                file: String::new(),
                line: None,
                message: format!(
                    "Coverage: {}/{} recommended scenario patterns covered for template '{}'",
                    covered,
                    recommendations.len(),
                    skill.name
                ),
            });
        }
    }

    issues
}

/// Extract recommended scenario descriptions from a skill template's content.
/// Looks for "## Recommended Scenarios" section and parses numbered items.
fn extract_recommended_scenarios(content: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut in_section = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## Recommended Scenarios") {
            in_section = true;
            continue;
        }
        if in_section {
            // Stop at next heading or section break
            if trimmed.starts_with("## ") || trimmed.starts_with("---") {
                break;
            }
            // Extract numbered items like "1. **text** — more text" or "- text"
            if let Some(item) = trimmed
                .strip_prefix(|c: char| c.is_ascii_digit())
                .and_then(|s| s.trim().strip_prefix(". "))
                .or_else(|| trimmed.strip_prefix("- "))
            {
                // Clean up markdown formatting: **text** → text
                let clean = item.replace("**", "").trim().to_string();
                if !clean.is_empty() {
                    result.push(clean);
                }
            }
        }
    }

    result
}
