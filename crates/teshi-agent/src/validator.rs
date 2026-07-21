//! Gherkin validation module.
//!
//! Provides functions to validate parsed feature files for common issues such
//! as missing Given/When/Then ordering, missing Examples tables, duplicate
//! scenario names, overly long scenarios, and cross-scenario data dependencies.

use std::collections::HashMap;

use teshi_core::gherkin::{BddProject, BddScenario, ScenarioKind};
use teshi_core::gherkin_lang::StepKeywordType;

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
/// - Scenarios must have a "When" and "Then" step (errors)
/// - Too many steps in a single scenario
/// - Scenario Outlines should have Examples tables
/// - Examples tables should have headers
/// - Duplicate scenario names
/// - Cross-scenario data dependencies (warnings)
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
                        severity: IssueSeverity::Error,
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
                        severity: IssueSeverity::Error,
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

            // Check cross-scenario data dependencies
            issues.extend(check_scenario_dependency(sc, &path));
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

/// Keywords in scenario names that may indicate a cross-scenario data dependency.
/// These patterns suggest the scenario assumes state established by another scenario.
static DEPENDENCY_NAME_PATTERNS: &[&str] = &[
    "continue after",
    "after login",
    "after sign",
    "after logging",
    "subsequent",
    "next step",
    "still logged",
    "step 2",
    "step 3",
    "继续",
    "接着",
    "然后",
    "后续",
    "第二步",
    "第三步",
];

/// Keywords in Given step text that may assume state from another scenario.
/// These patterns suggest the Given step relies on data set up by a different
/// scenario's When/Then chain rather than being self-contained.
static DEPENDENCY_GIVEN_PATTERNS: &[&str] = &[
    "still ",
    "continue ",
    "ongoing ",
    "current session",
    "current state",
    "same page",
    "仍然",
    "继续",
    "当前会话",
    "当前状态",
];

/// Check a single scenario for cross-scenario data dependencies.
///
/// Returns warnings if the scenario name or its Given steps contain keywords
/// suggesting it depends on state left by another scenario, making it
/// non-independently-executable.
fn check_scenario_dependency(scenario: &BddScenario, file: &str) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let name_lower = scenario.name.to_lowercase();

    // Check scenario name for dependency patterns
    for pattern in DEPENDENCY_NAME_PATTERNS {
        if name_lower.contains(pattern) {
            issues.push(ValidationIssue {
                severity: IssueSeverity::Warning,
                file: file.to_string(),
                line: Some(scenario.line_number),
                message: format!(
                    "Scenario '{}' may depend on state from another scenario (name contains '{}'). \
                     Each scenario must independently establish its own preconditions.",
                    scenario.name, pattern
                ),
            });
            break; // one warning per scenario is enough
        }
    }

    // Check Given steps for dependency patterns
    for step in &scenario.steps {
        if step.keyword_type != StepKeywordType::Given {
            continue;
        }
        let text_lower = step.text.to_lowercase();
        for pattern in DEPENDENCY_GIVEN_PATTERNS {
            if text_lower.contains(pattern) {
                issues.push(ValidationIssue {
                    severity: IssueSeverity::Warning,
                    file: file.to_string(),
                    line: Some(step.line_number),
                    message: format!(
                        "Given step '{}' may depend on state left by another scenario (contains '{}'). \
                         Each scenario must independently establish its own preconditions.",
                        step.text, pattern
                    ),
                });
                break; // one warning per step is enough
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

#[cfg(test)]
mod tests {
    use super::*;
    use teshi_core::gherkin::{BddFeature, BddProject, BddScenario, BddStep, ScenarioKind};
    use teshi_core::gherkin_lang::StepKeywordType;

    fn make_step(keyword_type: StepKeywordType, text: &str) -> BddStep {
        let keyword = match keyword_type {
            StepKeywordType::Given => "Given",
            StepKeywordType::When => "When",
            StepKeywordType::Then => "Then",
            _ => "And",
        };
        BddStep {
            keyword: keyword.into(),
            keyword_type,
            text: text.into(),
            line_number: 1,
            doc_string: None,
            data_table: None,
        }
    }

    fn make_scenario(name: &str, steps: Vec<BddStep>) -> BddScenario {
        BddScenario {
            name: name.into(),
            tags: vec![],
            kind: ScenarioKind::Scenario,
            steps,
            examples: vec![],
            line_number: 10,
        }
    }

    // ── check_scenario_dependency tests ──────────────────────────────────────

    #[test]
    fn dependency_clean_scenario_no_warnings() {
        let sc = make_scenario(
            "Successful login",
            vec![
                make_step(StepKeywordType::Given, "I am on the login page"),
                make_step(StepKeywordType::When, "I enter valid credentials"),
                make_step(StepKeywordType::Then, "I should see the dashboard"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "login.feature");
        assert!(
            issues.is_empty(),
            "clean scenario should have no dependency warnings: {:?}",
            issues
        );
    }

    #[test]
    fn dependency_scenario_name_after_login_triggers_warning() {
        let sc = make_scenario(
            "After login, access settings",
            vec![
                make_step(StepKeywordType::Given, "I am on the settings page"),
                make_step(StepKeywordType::When, "I click profile"),
                make_step(StepKeywordType::Then, "I should see my profile"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "login.feature");
        assert!(!issues.is_empty(), "should warn about 'after login'");
        assert!(issues[0].message.contains("after login"));
        assert_eq!(issues[0].severity, IssueSeverity::Warning);
    }

    #[test]
    fn dependency_scenario_name_continue_after_triggers_warning() {
        let sc = make_scenario(
            "Continue after signup",
            vec![
                make_step(StepKeywordType::Given, "I am on the welcome page"),
                make_step(StepKeywordType::When, "I complete the tutorial"),
                make_step(StepKeywordType::Then, "I should see the home page"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "onboarding.feature");
        assert!(!issues.is_empty(), "should warn about 'continue after'");
        assert!(issues[0].message.contains("continue after"));
    }

    #[test]
    fn dependency_scenario_name_subsequent_triggers_warning() {
        let sc = make_scenario(
            "Subsequent search",
            vec![
                make_step(StepKeywordType::Given, "I am on the search page"),
                make_step(StepKeywordType::When, "I enter a query"),
                make_step(StepKeywordType::Then, "I should see results"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "search.feature");
        assert!(!issues.is_empty(), "should warn about 'subsequent'");
    }

    #[test]
    fn dependency_scenario_name_next_step_triggers_warning() {
        let sc = make_scenario(
            "Next step: enter address",
            vec![
                make_step(StepKeywordType::Given, "I am on the checkout page"),
                make_step(StepKeywordType::When, "I enter my address"),
                make_step(StepKeywordType::Then, "I should see the shipping options"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "checkout.feature");
        assert!(!issues.is_empty(), "should warn about 'next step'");
    }

    #[test]
    fn dependency_scenario_name_still_logged_triggers_warning() {
        let sc = make_scenario(
            "Still logged in after 30 minutes",
            vec![
                make_step(StepKeywordType::Given, "I am on the dashboard"),
                make_step(StepKeywordType::When, "I refresh the page"),
                make_step(StepKeywordType::Then, "I should still see my data"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "session.feature");
        assert!(!issues.is_empty(), "should warn about 'still logged'");
    }

    #[test]
    fn dependency_given_still_triggers_warning() {
        let sc = make_scenario(
            "View profile",
            vec![
                make_step(StepKeywordType::Given, "I am still logged in"),
                make_step(StepKeywordType::When, "I navigate to profile"),
                make_step(StepKeywordType::Then, "I should see my account details"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "profile.feature");
        assert!(!issues.is_empty(), "should warn about 'still ' in Given");
        assert!(issues[0].message.contains("still "));
    }

    #[test]
    fn dependency_given_current_session_triggers_warning() {
        let sc = make_scenario(
            "Access admin panel",
            vec![
                make_step(StepKeywordType::Given, "the current session is valid"),
                make_step(StepKeywordType::When, "I visit /admin"),
                make_step(StepKeywordType::Then, "I should see the admin panel"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "admin.feature");
        assert!(!issues.is_empty(), "should warn about 'current session'");
    }

    #[test]
    fn dependency_given_same_page_triggers_warning() {
        let sc = make_scenario(
            "Continue editing",
            vec![
                make_step(StepKeywordType::Given, "I am on the same page"),
                make_step(StepKeywordType::When, "I click edit"),
                make_step(StepKeywordType::Then, "I should see the editor"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "editor.feature");
        assert!(!issues.is_empty(), "should warn about 'same page'");
    }

    #[test]
    fn dependency_given_continue_triggers_warning() {
        let sc = make_scenario(
            "Complete checkout",
            vec![
                make_step(StepKeywordType::Given, "I continue with the payment"),
                make_step(StepKeywordType::When, "I enter card details"),
                make_step(StepKeywordType::Then, "I should see a confirmation"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "payment.feature");
        assert!(!issues.is_empty(), "should warn about 'continue ' in Given");
    }

    #[test]
    fn dependency_given_ongoing_triggers_warning() {
        let sc = make_scenario(
            "Upload files",
            vec![
                make_step(StepKeywordType::Given, "the ongoing upload completes"),
                make_step(StepKeywordType::When, "I start a new upload"),
                make_step(StepKeywordType::Then, "I should see the progress bar"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "upload.feature");
        assert!(!issues.is_empty(), "should warn about 'ongoing ' in Given");
    }

    #[test]
    fn dependency_name_chinese_继续_triggers_warning() {
        let sc = make_scenario(
            "继续查看订单",
            vec![
                make_step(StepKeywordType::Given, "I am on the order page"),
                make_step(StepKeywordType::When, "I click details"),
                make_step(StepKeywordType::Then, "I should see order details"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "order.feature");
        assert!(!issues.is_empty(), "should warn about Chinese '继续'");
        assert!(issues[0].message.contains("继续"));
    }

    #[test]
    fn dependency_given_chinese_仍然_triggers_warning() {
        let sc = make_scenario(
            "查看个人资料",
            vec![
                make_step(StepKeywordType::Given, "我仍然处于登录状态"),
                make_step(StepKeywordType::When, "我导航到个人资料"),
                make_step(StepKeywordType::Then, "我应该看到我的信息"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "profile.feature");
        assert!(!issues.is_empty(), "should warn about Chinese '仍然'");
    }

    #[test]
    fn dependency_given_step_does_not_check_when_or_then() {
        let sc = make_scenario(
            "Search products",
            vec![
                make_step(StepKeywordType::Given, "I am on the home page"),
                make_step(StepKeywordType::When, "I still see the search bar"), // "still " but not a Given
                make_step(StepKeywordType::Then, "I should see results"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "search.feature");
        assert!(
            issues.is_empty(),
            "should not flag 'still ' in When/Then steps"
        );
    }

    #[test]
    fn dependency_scenario_name_without_keyword_no_warning() {
        let sc = make_scenario(
            "Search products",
            vec![
                make_step(StepKeywordType::Given, "I am on the home page"),
                make_step(StepKeywordType::When, "I search for a product"),
                make_step(StepKeywordType::Then, "I should see results"),
            ],
        );
        let issues = check_scenario_dependency(&sc, "search.feature");
        assert!(issues.is_empty(), "clean name should produce no warnings");
    }

    #[test]
    fn dependency_scenario_no_steps_no_warning() {
        let sc = make_scenario("Empty scenario", vec![]);
        let issues = check_scenario_dependency(&sc, "empty.feature");
        assert!(
            issues.is_empty(),
            "scenario with no steps should not trigger"
        );
    }

    // ── validate_project severity tests ──────────────────────────────────────

    #[test]
    fn validate_missing_when_is_error() {
        let project = make_project_with_scenario(
            "No When",
            vec![
                make_step(StepKeywordType::Given, "I am on the page"),
                make_step(StepKeywordType::Then, "I should see something"),
            ],
        );
        let issues = validate_project(&project);
        let when_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("no 'When' step"))
            .collect();
        assert!(!when_issues.is_empty(), "should report missing When");
        assert_eq!(
            when_issues[0].severity,
            IssueSeverity::Error,
            "missing When must be Error"
        );
    }

    #[test]
    fn validate_missing_then_is_error() {
        let project = make_project_with_scenario(
            "No Then",
            vec![
                make_step(StepKeywordType::Given, "I am on the page"),
                make_step(StepKeywordType::When, "I do something"),
            ],
        );
        let issues = validate_project(&project);
        let then_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("no 'Then' step"))
            .collect();
        assert!(!then_issues.is_empty(), "should report missing Then");
        assert_eq!(
            then_issues[0].severity,
            IssueSeverity::Error,
            "missing Then must be Error"
        );
    }

    #[test]
    fn validate_single_given_only_no_when_then_warning() {
        // A single "Given" step (only 1 step total) does not trigger When/Then errors
        // because the threshold is sc.steps.len() >= 2
        let project = make_project_with_scenario(
            "Just Given",
            vec![make_step(StepKeywordType::Given, "I am on the page")],
        );
        let issues = validate_project(&project);
        let when_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("no 'When' step"))
            .collect();
        let then_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("no 'Then' step"))
            .collect();
        assert!(
            when_issues.is_empty(),
            "single step should not trigger missing When"
        );
        assert!(
            then_issues.is_empty(),
            "single step should not trigger missing Then"
        );
    }

    #[test]
    fn validate_valid_gwt_no_errors() {
        let project = make_project_with_scenario(
            "Full chain",
            vec![
                make_step(StepKeywordType::Given, "I am on the page"),
                make_step(StepKeywordType::When, "I do something"),
                make_step(StepKeywordType::Then, "I should see something"),
            ],
        );
        let issues = validate_project(&project);
        let when_errors: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("no 'When' step"))
            .collect();
        let then_errors: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("no 'Then' step"))
            .collect();
        assert!(
            when_errors.is_empty(),
            "valid GWT should not report missing When"
        );
        assert!(
            then_errors.is_empty(),
            "valid GWT should not report missing Then"
        );
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_project_with_scenario(name: &str, steps: Vec<BddStep>) -> BddProject {
        let scenario = make_scenario(name, steps);
        BddProject {
            root_dir: std::path::PathBuf::from("/fake"),
            features: vec![BddFeature {
                file_path: std::path::PathBuf::from("test.feature"),
                name: "Test Feature".into(),
                language: "en".into(),
                tags: vec![],
                description: vec![],
                background: None,
                rules: vec![],
                scenarios: vec![scenario],
                line_count: 100,
            }],
        }
    }
}
