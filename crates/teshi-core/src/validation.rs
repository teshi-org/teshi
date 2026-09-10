//! Source-aware Gherkin validation and diagnostics.
//!
//! The parser in [`crate::gherkin`] intentionally remains permissive so an
//! editor can render an incomplete buffer.  This module is the strict,
//! source-aware companion: it diagnoses lines which the parser cannot safely
//! represent and validates the resulting AST without performing I/O.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::gherkin::{BddFeature, BddProject, BddScenario, ScenarioKind, parse_feature};
use crate::gherkin_lang::{GherkinLanguage, GherkinLanguages, StepKeywordType, StructuralType};

/// Severity used by a [`ValidationDiagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Suggestion,
}

impl DiagnosticSeverity {
    fn sort_rank(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warning => 1,
            Self::Suggestion => 2,
        }
    }
}

/// A stable machine-readable validation code.
pub mod diagnostic_codes {
    pub const MISSING_FEATURE_HEADER: &str = "missing_feature_header";
    pub const EMPTY_FEATURE_NAME: &str = "empty_feature_name";
    pub const MALFORMED_STRUCTURAL_HEADER: &str = "malformed_structural_header";
    pub const DUPLICATE_FEATURE_HEADER: &str = "duplicate_feature_header";
    pub const EMPTY_RULE_NAME: &str = "empty_rule_name";
    pub const EMPTY_SCENARIO_NAME: &str = "empty_scenario_name";
    pub const SCENARIO_WITHOUT_STEPS: &str = "scenario_without_steps";
    pub const SCENARIO_OUTLINE_MISSING_EXAMPLES: &str = "scenario_outline_missing_examples";
    pub const EXAMPLES_WITHOUT_OUTLINE: &str = "examples_without_outline";
    pub const EXAMPLES_MISSING_HEADERS: &str = "examples_missing_headers";
    pub const MISSING_STEP_SEPARATOR: &str = "missing_step_separator";
    pub const UNRECOGNIZED_EXECUTABLE_LINE: &str = "unrecognized_executable_line";
    pub const STEP_OUTSIDE_SCENARIO: &str = "step_outside_scenario";
    pub const EMPTY_STEP: &str = "empty_step";
    pub const UNTERMINATED_DOC_STRING: &str = "unterminated_doc_string";
    pub const DUPLICATE_SCENARIO_NAME: &str = "duplicate_scenario_name";
    pub const MISSING_WHEN: &str = "missing_when";
    pub const MISSING_THEN: &str = "missing_then";
    pub const SCENARIO_STARTS_WITHOUT_GIVEN: &str = "scenario_starts_without_given";
    pub const SCENARIO_TOO_MANY_STEPS: &str = "scenario_too_many_steps";
    pub const CROSS_SCENARIO_DEPENDENCY: &str = "cross_scenario_dependency";
}

/// One source location and actionable validation finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationDiagnostic {
    /// Feature path as supplied by the caller, rendered with `/` separators.
    pub path: String,
    /// One-based source line.
    pub line: usize,
    /// One-based Unicode character column.
    pub column: usize,
    pub severity: DiagnosticSeverity,
    /// Stable snake_case code suitable for automation.
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// Aggregate counts for a validation report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub errors: usize,
    pub warnings: usize,
    pub suggestions: usize,
}

/// Result of validating one or more Feature sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    /// The checked source paths in deterministic order.
    pub scope: Vec<String>,
    pub summary: ValidationSummary,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        self.summary.errors == 0
    }

    pub fn has_errors(&self) -> bool {
        !self.is_valid()
    }
}

/// Borrowed source input for the pure multi-file validation API.
#[derive(Debug, Clone, Copy)]
pub struct FeatureSource<'a> {
    pub path: &'a Path,
    pub content: &'a str,
}

/// Validate one raw Feature source without filesystem access.
pub fn validate_feature_source(content: &str, path: impl AsRef<Path>) -> ValidationReport {
    let path = path.as_ref();
    let path_text = display_path(path);
    let mut diagnostics = Vec::new();
    let language_code = GherkinLanguages::detect_from_content(content);
    let language = GherkinLanguages::global().get(language_code);

    scan_source(content, path_text.as_str(), language, &mut diagnostics);

    let feature = parse_feature(content, path.to_path_buf());
    validate_feature_ast(&feature, path_text.as_str(), &mut diagnostics, false);

    build_report(vec![path_text], diagnostics)
}

/// Validate borrowed Feature sources without reading files or invoking a shell.
pub fn validate_feature_sources<'a, I>(sources: I) -> ValidationReport
where
    I: IntoIterator<Item = FeatureSource<'a>>,
{
    let mut diagnostics = Vec::new();
    let mut scope = Vec::new();

    for source in sources {
        let path_text = display_path(source.path);
        let language_code = GherkinLanguages::detect_from_content(source.content);
        let language = GherkinLanguages::global().get(language_code);
        scan_source(
            source.content,
            path_text.as_str(),
            language,
            &mut diagnostics,
        );
        let feature = parse_feature(source.content, source.path.to_path_buf());
        validate_feature_ast(&feature, path_text.as_str(), &mut diagnostics, false);
        scope.push(path_text);
    }

    scope.sort();
    scope.dedup();
    build_report(scope, diagnostics)
}

/// Validate the already parsed project. This is useful to callers that have
/// retained ASTs, but source-aware checks require [`validate_feature_source`].
pub fn validate_project(project: &BddProject) -> ValidationReport {
    let mut diagnostics = Vec::new();
    let mut scope = Vec::new();
    for feature in &project.features {
        let path = display_path(&feature.file_path);
        validate_feature_ast(feature, path.as_str(), &mut diagnostics, true);
        scope.push(path);
    }
    scope.sort();
    scope.dedup();
    build_report(scope, diagnostics)
}

fn build_report(
    mut scope: Vec<String>,
    mut diagnostics: Vec<ValidationDiagnostic>,
) -> ValidationReport {
    scope.sort();
    scope.dedup();
    diagnostics.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.column.cmp(&b.column))
            .then(a.severity.sort_rank().cmp(&b.severity.sort_rank()))
            .then(a.code.cmp(&b.code))
            .then(a.message.cmp(&b.message))
    });
    diagnostics.dedup_by(|a, b| {
        a.path == b.path && a.line == b.line && a.column == b.column && a.code == b.code
    });

    let mut summary = ValidationSummary::default();
    for diagnostic in &diagnostics {
        match diagnostic.severity {
            DiagnosticSeverity::Error => summary.errors += 1,
            DiagnosticSeverity::Warning => summary.warnings += 1,
            DiagnosticSeverity::Suggestion => summary.suggestions += 1,
        }
    }

    ValidationReport {
        scope,
        summary,
        diagnostics,
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    Outside,
    Feature,
    Rule,
    Background,
    Scenario,
    ScenarioOutline,
    Examples,
}

fn scan_source(
    content: &str,
    path: &str,
    language: &GherkinLanguage,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let lines: Vec<&str> = content.lines().collect();
    let mut block = Block::Outside;
    let mut feature_seen = false;
    let mut steps_started = false;
    let mut previous_step = false;
    let mut table_attachment = false;
    let mut doc_string_marker: Option<&str> = None;
    let mut first_content_line = None;

    for (index, line) in lines.iter().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();

        if let Some(marker) = doc_string_marker {
            if trimmed.starts_with(marker) {
                doc_string_marker = None;
            }
            previous_step = false;
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if first_content_line.is_none() && !trimmed.starts_with('@') {
            first_content_line = Some(line_number);
        }
        if trimmed.starts_with('@') {
            // Tags are valid before the next structural block. They must not
            // turn a partial editor buffer into an executable-line error.
            previous_step = false;
            continue;
        }

        if table_attachment {
            if trimmed.starts_with('|') {
                continue;
            }
            table_attachment = false;
        }

        if previous_step
            && (trimmed.starts_with("|")
                || trimmed.starts_with("\"\"\"")
                || trimmed.starts_with("```"))
        {
            if trimmed.starts_with('|') {
                table_attachment = true;
            } else {
                doc_string_marker = Some(&trimmed[..3]);
            }
            previous_step = false;
            continue;
        }

        if let Some((keyword, structural_type, _rest)) =
            malformed_structural_prefix(language, trimmed)
        {
            if structural_type == StructuralType::Feature {
                feature_seen = true;
            }
            block = match structural_type {
                StructuralType::Feature => Block::Feature,
                StructuralType::Rule => Block::Rule,
                StructuralType::Background => Block::Background,
                StructuralType::Scenario => Block::Scenario,
                StructuralType::ScenarioOutline => Block::ScenarioOutline,
                StructuralType::Examples => Block::Examples,
            };
            steps_started = false;
            push_diagnostic(
                diagnostics,
                path,
                line_number,
                leading_column(line) + keyword.chars().count(),
                DiagnosticSeverity::Error,
                diagnostic_codes::MALFORMED_STRUCTURAL_HEADER,
                format!(
                    "Structural keyword '{}' must be followed by a space",
                    keyword
                ),
                Some(format!("Insert a space after '{}'", keyword)),
            );
            previous_step = false;
            continue;
        }

        if let Some((keyword, structural_type)) = language.match_structural_prefix(trimmed) {
            let rest = trimmed[keyword.len()..].trim();
            match structural_type {
                StructuralType::Feature => {
                    if feature_seen {
                        push_diagnostic(
                            diagnostics,
                            path,
                            line_number,
                            1,
                            DiagnosticSeverity::Error,
                            diagnostic_codes::DUPLICATE_FEATURE_HEADER,
                            "Feature header appears more than once",
                            None,
                        );
                    } else {
                        feature_seen = true;
                        block = Block::Feature;
                        if rest.is_empty() {
                            push_diagnostic(
                                diagnostics,
                                path,
                                line_number,
                                keyword.chars().count() + 1,
                                DiagnosticSeverity::Error,
                                diagnostic_codes::EMPTY_FEATURE_NAME,
                                "Feature header must have a name",
                                Some("Add a Feature name after the colon".to_string()),
                            );
                        }
                    }
                }
                StructuralType::Rule => {
                    block = Block::Rule;
                    if rest.is_empty() {
                        push_diagnostic(
                            diagnostics,
                            path,
                            line_number,
                            leading_column(line) + keyword.chars().count(),
                            DiagnosticSeverity::Error,
                            diagnostic_codes::EMPTY_RULE_NAME,
                            "Rule header must have a name",
                            Some("Add a Rule name after the colon".to_string()),
                        );
                    }
                }
                StructuralType::Background => {
                    block = Block::Background;
                    steps_started = false;
                }
                StructuralType::Scenario => {
                    block = Block::Scenario;
                    steps_started = false;
                    if rest.is_empty() {
                        push_diagnostic(
                            diagnostics,
                            path,
                            line_number,
                            keyword.chars().count() + 1,
                            DiagnosticSeverity::Error,
                            diagnostic_codes::EMPTY_SCENARIO_NAME,
                            "Scenario header must have a name",
                            Some("Add a Scenario name after the colon".to_string()),
                        );
                    }
                }
                StructuralType::ScenarioOutline => {
                    block = Block::ScenarioOutline;
                    steps_started = false;
                    if rest.is_empty() {
                        push_diagnostic(
                            diagnostics,
                            path,
                            line_number,
                            keyword.chars().count() + 1,
                            DiagnosticSeverity::Error,
                            diagnostic_codes::EMPTY_SCENARIO_NAME,
                            "Scenario Outline header must have a name",
                            Some("Add a Scenario Outline name after the colon".to_string()),
                        );
                    }
                }
                StructuralType::Examples => {
                    if block != Block::ScenarioOutline {
                        push_diagnostic(
                            diagnostics,
                            path,
                            line_number,
                            1,
                            DiagnosticSeverity::Error,
                            diagnostic_codes::EXAMPLES_WITHOUT_OUTLINE,
                            "Examples must belong to a Scenario Outline",
                            None,
                        );
                    }
                    block = Block::Examples;
                    steps_started = false;
                }
            }
            previous_step = false;
            continue;
        }

        if let Some((keyword, _keyword_type, has_separator, rest)) = step_prefix(language, trimmed)
        {
            if !has_separator {
                let suggestion = format!("{} {}", keyword, rest.trim_start());
                push_diagnostic(
                    diagnostics,
                    path,
                    line_number,
                    leading_column(line) + keyword.chars().count(),
                    DiagnosticSeverity::Error,
                    diagnostic_codes::MISSING_STEP_SEPARATOR,
                    format!("Step keyword '{}' must be followed by a space", keyword),
                    Some(suggestion),
                );
                previous_step = false;
                continue;
            }

            if !matches!(
                block,
                Block::Scenario | Block::ScenarioOutline | Block::Background
            ) {
                push_diagnostic(
                    diagnostics,
                    path,
                    line_number,
                    1,
                    DiagnosticSeverity::Error,
                    diagnostic_codes::STEP_OUTSIDE_SCENARIO,
                    "A step must be inside a Scenario, Scenario Outline, or Background",
                    None,
                );
            }
            if rest.trim_matches([' ', '\u{3000}']).is_empty() {
                push_diagnostic(
                    diagnostics,
                    path,
                    line_number,
                    leading_column(line) + keyword.chars().count(),
                    DiagnosticSeverity::Error,
                    diagnostic_codes::EMPTY_STEP,
                    "Step must have text after its keyword",
                    None,
                );
            }
            steps_started = true;
            previous_step = true;
            continue;
        }

        if block == Block::Examples {
            if trimmed.starts_with('|') {
                continue;
            }
            if language.is_structural(trimmed) {
                continue;
            }
        }

        // Description prose is legal before the first step. Once executable
        // content has started, silently ignoring a line would lose intent.
        if matches!(
            block,
            Block::Scenario | Block::ScenarioOutline | Block::Background
        ) && steps_started
        {
            push_diagnostic(
                diagnostics,
                path,
                line_number,
                leading_column(line),
                DiagnosticSeverity::Error,
                diagnostic_codes::UNRECOGNIZED_EXECUTABLE_LINE,
                "Unrecognized text appears in the executable step region",
                Some(
                    "Use a supported step keyword or move this text into a description".to_string(),
                ),
            );
        }
        previous_step = false;
    }

    if let Some(marker) = doc_string_marker {
        let line_number = lines.len().max(1);
        push_diagnostic(
            diagnostics,
            path,
            line_number,
            1,
            DiagnosticSeverity::Error,
            diagnostic_codes::UNTERMINATED_DOC_STRING,
            format!("DocString starting with {marker} is not terminated"),
            None,
        );
    }

    if !feature_seen {
        let line_number = first_content_line.unwrap_or(1);
        push_diagnostic(
            diagnostics,
            path,
            line_number,
            1,
            DiagnosticSeverity::Error,
            diagnostic_codes::MISSING_FEATURE_HEADER,
            "Feature source must contain a Feature header",
            Some("Add a Feature: header before the Feature description".to_string()),
        );
    }
}

fn leading_column(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count() + 1
}

/// Find the longest dialect keyword prefix and report whether its separator is
/// valid. The parser's `match_step_prefix` intentionally returns `None` for a
/// malformed separator, so validation must inspect this case separately.
fn step_prefix<'a, 'b>(
    language: &'b GherkinLanguage,
    trimmed: &'a str,
) -> Option<(&'b str, StepKeywordType, bool, &'a str)> {
    let mut matches: Vec<(&str, StepKeywordType, &str)> = language
        .all_step_keywords()
        .iter()
        .filter_map(|keyword| {
            let rest = trimmed.strip_prefix(keyword.as_str())?;
            Some((keyword.as_str(), language.classify_step(keyword)?, rest))
        })
        .collect();
    matches.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(b.0)));
    let (keyword, kind, rest) = matches.into_iter().next()?;
    let has_separator = rest.is_empty()
        || rest
            .chars()
            .next()
            .is_some_and(|c| matches!(c, ' ' | '\u{3000}'));
    Some((keyword, kind, has_separator, rest))
}

fn malformed_structural_prefix<'a, 'b>(
    language: &'b GherkinLanguage,
    trimmed: &'a str,
) -> Option<(&'b str, StructuralType, &'a str)> {
    let mut keywords: Vec<&'b String> = language.all_structural_keywords().iter().collect();
    keywords.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    for keyword in keywords {
        let Some(rest) = trimmed.strip_prefix(keyword.as_str()) else {
            continue;
        };
        if rest.is_empty() || rest.chars().next().is_some_and(char::is_whitespace) {
            continue;
        }
        let probe = format!("{keyword} ");
        let Some((_, structural_type)) = language.match_structural_prefix(&probe) else {
            continue;
        };
        return Some((keyword.as_str(), structural_type, rest));
    }
    None
}

fn validate_feature_ast(
    feature: &BddFeature,
    path: &str,
    diagnostics: &mut Vec<ValidationDiagnostic>,
    check_feature_name: bool,
) {
    if check_feature_name && feature.name.trim().is_empty() {
        // The source scanner reports the precise header location. This AST
        // fallback keeps `validate_project` useful for parsed callers.
        push_diagnostic(
            diagnostics,
            path,
            1,
            1,
            DiagnosticSeverity::Error,
            diagnostic_codes::MISSING_FEATURE_HEADER,
            "Feature source must contain a named Feature",
            None,
        );
    }

    let scenarios = feature.all_scenarios();
    let mut names: HashMap<&str, (usize, usize)> = HashMap::new();
    for scenario in &scenarios {
        let entry = names
            .entry(scenario.name.as_str())
            .or_insert((0, scenario.line_number));
        entry.0 += 1;
        validate_scenario(scenario, path, diagnostics);
    }
    for (name, (count, line)) in names {
        if count > 1 {
            push_diagnostic(
                diagnostics,
                path,
                line,
                1,
                DiagnosticSeverity::Error,
                diagnostic_codes::DUPLICATE_SCENARIO_NAME,
                format!("Scenario name '{}' appears {} times", name, count),
                None,
            );
        }
    }

    if let Some(background) = &feature.background
        && background.steps.is_empty()
    {
        push_diagnostic(
            diagnostics,
            path,
            background.line_number,
            1,
            DiagnosticSeverity::Warning,
            diagnostic_codes::SCENARIO_WITHOUT_STEPS,
            "Background has no steps",
            None,
        );
    }
}

fn validate_scenario(
    scenario: &BddScenario,
    path: &str,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if scenario.steps.is_empty() {
        push_diagnostic(
            diagnostics,
            path,
            scenario.line_number,
            1,
            DiagnosticSeverity::Error,
            diagnostic_codes::SCENARIO_WITHOUT_STEPS,
            format!("Scenario '{}' has no recognized steps", scenario.name),
            None,
        );
    }

    let types: Vec<StepKeywordType> = scenario
        .steps
        .iter()
        .map(|step| step.keyword_type)
        .collect();
    if let Some(first) = types.first()
        && *first != StepKeywordType::Given
    {
        push_diagnostic(
            diagnostics,
            path,
            scenario.line_number,
            1,
            DiagnosticSeverity::Warning,
            diagnostic_codes::SCENARIO_STARTS_WITHOUT_GIVEN,
            format!(
                "Scenario '{}' does not start with a Given step",
                scenario.name
            ),
            None,
        );
    }
    if scenario.steps.len() >= 2 && !types.contains(&StepKeywordType::When) {
        push_diagnostic(
            diagnostics,
            path,
            scenario.line_number,
            1,
            DiagnosticSeverity::Warning,
            diagnostic_codes::MISSING_WHEN,
            format!("Scenario '{}' has no When step", scenario.name),
            None,
        );
    }
    if scenario.steps.len() >= 2 && !types.contains(&StepKeywordType::Then) {
        push_diagnostic(
            diagnostics,
            path,
            scenario.line_number,
            1,
            DiagnosticSeverity::Warning,
            diagnostic_codes::MISSING_THEN,
            format!("Scenario '{}' has no Then step", scenario.name),
            None,
        );
    }
    if scenario.steps.len() > 10 {
        push_diagnostic(
            diagnostics,
            path,
            scenario.line_number,
            1,
            DiagnosticSeverity::Suggestion,
            diagnostic_codes::SCENARIO_TOO_MANY_STEPS,
            format!(
                "Scenario '{}' has {} steps; consider splitting it",
                scenario.name,
                scenario.steps.len()
            ),
            None,
        );
    }

    if matches!(scenario.kind, ScenarioKind::ScenarioOutline) && scenario.examples.is_empty() {
        push_diagnostic(
            diagnostics,
            path,
            scenario.line_number,
            1,
            DiagnosticSeverity::Error,
            diagnostic_codes::SCENARIO_OUTLINE_MISSING_EXAMPLES,
            format!("Scenario Outline '{}' has no Examples table", scenario.name),
            None,
        );
    }
    for examples in &scenario.examples {
        if examples.headers.is_empty() {
            push_diagnostic(
                diagnostics,
                path,
                examples.line_number,
                1,
                DiagnosticSeverity::Error,
                diagnostic_codes::EXAMPLES_MISSING_HEADERS,
                format!("Examples table in '{}' has no headers", scenario.name),
                None,
            );
        }
    }

    let name_lower = scenario.name.to_lowercase();
    let name_patterns = [
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
    if let Some(pattern) = name_patterns
        .iter()
        .find(|pattern| name_lower.contains(**pattern))
    {
        push_diagnostic(
            diagnostics,
            path,
            scenario.line_number,
            1,
            DiagnosticSeverity::Warning,
            diagnostic_codes::CROSS_SCENARIO_DEPENDENCY,
            format!(
                "Scenario '{}' may depend on another scenario (contains '{}')",
                scenario.name, pattern
            ),
            None,
        );
    }
    let given_patterns = [
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
    for step in &scenario.steps {
        if step.keyword_type != StepKeywordType::Given {
            continue;
        }
        let text_lower = step.text.to_lowercase();
        if let Some(_pattern) = given_patterns
            .iter()
            .find(|pattern| text_lower.contains(**pattern))
        {
            push_diagnostic(
                diagnostics,
                path,
                step.line_number,
                1,
                DiagnosticSeverity::Warning,
                diagnostic_codes::CROSS_SCENARIO_DEPENDENCY,
                format!("Given step '{}' may depend on another scenario", step.text),
                None,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_diagnostic(
    diagnostics: &mut Vec<ValidationDiagnostic>,
    path: &str,
    line: usize,
    column: usize,
    severity: DiagnosticSeverity,
    code: &str,
    message: impl Into<String>,
    suggestion: Option<String>,
) {
    diagnostics.push(ValidationDiagnostic {
        path: path.to_string(),
        line,
        column,
        severity,
        code: code.to_string(),
        message: message.into(),
        suggestion,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(report: &ValidationReport) -> Vec<&str> {
        report.diagnostics.iter().map(|d| d.code.as_str()).collect()
    }

    #[test]
    fn valid_english_feature_has_no_errors() {
        let source = "Feature: Login\n  Scenario: Success\n    Given the login page is open\n    When the user logs in\n    Then the dashboard is visible\n";
        let report = validate_feature_source(source, "login.feature");
        assert!(!report.has_errors(), "{report:?}");
    }

    #[test]
    fn missing_separator_is_reported_with_unicode_column_and_suggestion() {
        let source = "# language: zh-CN\n功能: 登录\n  场景: 成功\n    当用户登录\n";
        let report = validate_feature_source(source, "login.feature");
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|d| d.code == diagnostic_codes::MISSING_STEP_SEPARATOR)
            .expect("missing separator diagnostic");
        assert_eq!(diagnostic.line, 4);
        assert_eq!(diagnostic.column, 6);
        assert_eq!(diagnostic.suggestion.as_deref(), Some("当 用户登录"));
    }

    #[test]
    fn repeated_given_and_and_steps_are_valid_but_may_warn() {
        let source = "Feature: Setup\n  Scenario: Preconditions only\n    Given the account exists\n    And the account is active\n";
        let report = validate_feature_source(source, "setup.feature");
        assert!(
            !report.has_errors(),
            "legal Gherkin was blocked: {report:?}"
        );
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == diagnostic_codes::MISSING_WHEN
                && diagnostic.severity == DiagnosticSeverity::Warning
        }));
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == diagnostic_codes::MISSING_THEN
                && diagnostic.severity == DiagnosticSeverity::Warning
        }));
    }

    #[test]
    fn valid_scenario_description_is_not_an_error() {
        let source = "Feature: Login\n  Scenario: Success\n    The user is already registered.\n    Given the login page is open\n    When the user logs in\n    Then the dashboard is visible\n";
        let report = validate_feature_source(source, "login.feature");
        assert!(!codes(&report).contains(&diagnostic_codes::UNRECOGNIZED_EXECUTABLE_LINE));
    }

    #[test]
    fn executable_region_text_is_reported() {
        let source = "Feature: Login\n  Scenario: Success\n    Given the login page is open\n    this line is not a step\n    Then the dashboard is visible\n";
        let report = validate_feature_source(source, "login.feature");
        assert!(codes(&report).contains(&diagnostic_codes::UNRECOGNIZED_EXECUTABLE_LINE));
    }

    #[test]
    fn tables_and_doc_strings_are_accepted() {
        let source = "Feature: Data\n  Scenario: Table\n    Given data exists\n      | name | value |\n      | one  | 1     |\n    When it is read\n    Then the value is returned\n  Scenario: Doc\n    Given a document\n      \"\"\"\n      body\n      \"\"\"\n    When it is read\n    Then the body is returned\n";
        let report = validate_feature_source(source, "data.feature");
        assert!(!codes(&report).contains(&diagnostic_codes::UNRECOGNIZED_EXECUTABLE_LINE));
    }

    #[test]
    fn malformed_source_keeps_permissive_ast_but_has_diagnostic() {
        let source = "# language: zh-CN\n功能: 登录\n  场景: 成功\n    当用户登录\n";
        let feature = parse_feature(source, std::path::PathBuf::from("login.feature"));
        assert!(feature.scenarios[0].steps.is_empty());
        let report = validate_feature_source(source, "login.feature");
        assert!(report.has_errors());
        assert!(codes(&report).contains(&diagnostic_codes::MISSING_STEP_SEPARATOR));
    }

    #[test]
    fn reports_are_sorted_and_counted() {
        let source = "Feature: Login\n  Scenario: S\n    Given x\n    stray\n";
        let report = validate_feature_source(source, "login.feature");
        assert_eq!(report.summary.errors, report.diagnostics.len());
        assert!(
            report
                .diagnostics
                .windows(2)
                .all(|pair| (pair[0].line, pair[0].column) <= (pair[1].line, pair[1].column))
        );
    }

    #[test]
    fn project_validation_does_not_need_filesystem_access() {
        let first = std::path::PathBuf::from("a.feature");
        let second = std::path::PathBuf::from("b.feature");
        let report = validate_feature_sources([
            FeatureSource {
                path: &first,
                content: "Feature: A\n  Scenario: S\n    Given x\n",
            },
            FeatureSource {
                path: &second,
                content: "Feature: B\n  Scenario: S\n    Given y\n",
            },
        ]);
        assert_eq!(report.scope, vec!["a.feature", "b.feature"]);
        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn malformed_structural_separator_is_reported() {
        let source = "Feature: Login\n  Scenario:Success\n    Given the login page is open\n";
        let report = validate_feature_source(source, "login.feature");
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|d| d.code == diagnostic_codes::MALFORMED_STRUCTURAL_HEADER)
            .expect("malformed structural header diagnostic");
        assert_eq!(diagnostic.line, 2);
        assert_eq!(diagnostic.column, 12);
    }

    #[test]
    fn french_longest_step_keyword_wins_for_separator_diagnostic() {
        let source =
            "# language: fr\nFonctionnalité: Connexion\n  Scénario: Succès\n    Étant donné queX\n";
        let report = validate_feature_source(source, "login.feature");
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|d| d.code == diagnostic_codes::MISSING_STEP_SEPARATOR)
            .expect("missing separator diagnostic");
        assert_eq!(diagnostic.suggestion.as_deref(), Some("Étant donné que X"));
        assert_eq!(diagnostic.column, 20);
    }

    #[test]
    fn empty_rule_name_is_reported() {
        let source =
            "Feature: Login\n  Rule:\n    Scenario: Success\n      Given the login page is open\n";
        let report = validate_feature_source(source, "login.feature");
        assert!(codes(&report).contains(&diagnostic_codes::EMPTY_RULE_NAME));
    }
}
