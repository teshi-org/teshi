//! Lightweight Gherkin AST types and parser.
//!
//! Converts raw `.feature` file content into a structured tree: `BddProject` → `BddFeature` →
//! `BddScenario` → `BddStep`. Each node records its source `line_number` (1-based) for mapping
//! back to the editor buffer.

use std::path::PathBuf;

use crate::gherkin_keywords::STEP_KEYWORDS;

// ── AST types ────────────────────────────────────────────────────────────────

/// Root container for all loaded feature files.
#[derive(Debug, Clone)]
pub struct BddProject {
    pub root_dir: PathBuf,
    pub features: Vec<BddFeature>,
}

/// One `.feature` file parsed into structure.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BddFeature {
    pub file_path: PathBuf,
    pub name: String,
    pub tags: Vec<String>,
    pub description: Vec<String>,
    pub background: Option<BddBackground>,
    pub scenarios: Vec<BddScenario>,
    pub line_count: usize,
}

/// `Background:` block — shared steps prepended to every scenario.
#[derive(Debug, Clone)]
pub struct BddBackground {
    pub steps: Vec<BddStep>,
    /// 1-based line number of the `Background:` keyword.
    pub line_number: usize,
}

/// A `Scenario:` or `Scenario Outline:` block.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BddScenario {
    pub name: String,
    pub tags: Vec<String>,
    pub kind: ScenarioKind,
    pub steps: Vec<BddStep>,
    pub examples: Vec<ExamplesTable>,
    /// 1-based line number of the `Scenario:` / `Scenario Outline:` keyword.
    pub line_number: usize,
}

/// A single step line (e.g. `Given I am on the login page`).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BddStep {
    /// Leading keyword: `Given`, `When`, `Then`, `And`, or `But`.
    pub keyword: String,
    /// Body text after the keyword (trimmed of the leading space).
    pub text: String,
    /// 1-based line number.
    pub line_number: usize,
}

/// `Examples:` data table attached to a `Scenario Outline`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ExamplesTable {
    pub tags: Vec<String>,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// 1-based line number of the `Examples:` keyword.
    pub line_number: usize,
}

/// Distinguishes regular scenarios from outline templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioKind {
    Scenario,
    ScenarioOutline,
}

// ── Parser ───────────────────────────────────────────────────────────────────

/// Parses a single `.feature` file from its raw text content.
pub fn parse_feature(content: &str, file_path: PathBuf) -> BddFeature {
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();
    let mut idx = 0;

    let feature_tags;
    let mut feature_name = String::new();
    let mut feature_description = Vec::new();
    let mut background: Option<BddBackground> = None;
    let mut scenarios: Vec<BddScenario> = Vec::new();

    // Collect tags before `Feature:`
    idx = skip_blank_and_comments(&lines, idx);
    feature_tags = collect_tags(&lines, &mut idx);

    // Find `Feature:` line
    if idx < lines.len() {
        let trimmed = lines[idx].trim();
        if let Some(rest) = trimmed.strip_prefix("Feature:") {
            feature_name = rest.trim().to_string();
            idx += 1;
        }
    }

    // Feature description: free text before the first structural keyword
    while idx < lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || (!is_structural_keyword(trimmed) && !trimmed.starts_with('@'))
        {
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                feature_description.push(trimmed.to_string());
            }
            idx += 1;
        } else {
            break;
        }
    }

    // Parse body: Background, Scenarios
    while idx < lines.len() {
        idx = skip_blank_and_comments(&lines, idx);
        if idx >= lines.len() {
            break;
        }

        // Collect tags for the next block
        let block_tags = collect_tags(&lines, &mut idx);
        if idx >= lines.len() {
            break;
        }

        let trimmed = lines[idx].trim();

        if trimmed.starts_with("Background:") {
            let bg_line = idx + 1; // 1-based
            idx += 1;
            let steps = parse_steps(&lines, &mut idx);
            background = Some(BddBackground {
                steps,
                line_number: bg_line,
            });
        } else if let Some(rest) = trimmed.strip_prefix("Scenario Outline:") {
            let sc_line = idx + 1;
            let sc_name = rest.trim().to_string();
            idx += 1;
            let steps = parse_steps(&lines, &mut idx);
            let examples = parse_examples_blocks(&lines, &mut idx);
            scenarios.push(BddScenario {
                name: sc_name,
                tags: block_tags,
                kind: ScenarioKind::ScenarioOutline,
                steps,
                examples,
                line_number: sc_line,
            });
        } else if let Some(rest) = trimmed.strip_prefix("Scenario:") {
            let sc_line = idx + 1;
            let sc_name = rest.trim().to_string();
            idx += 1;
            let steps = parse_steps(&lines, &mut idx);
            let examples = parse_examples_blocks(&lines, &mut idx);
            scenarios.push(BddScenario {
                name: sc_name,
                tags: block_tags,
                kind: ScenarioKind::Scenario,
                steps,
                examples,
                line_number: sc_line,
            });
        } else {
            // Unknown line — skip to avoid infinite loop
            idx += 1;
        }
    }

    BddFeature {
        file_path,
        name: feature_name,
        tags: feature_tags,
        description: feature_description,
        background,
        scenarios,
        line_count,
    }
}

/// Parses an entire directory of `.feature` files into a `BddProject`.
pub fn parse_project(root_dir: &std::path::Path) -> BddProject {
    let mut feature_paths: Vec<PathBuf> = Vec::new();
    collect_feature_files(root_dir, &mut feature_paths);
    feature_paths.sort();

    let features: Vec<BddFeature> = feature_paths
        .into_iter()
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            Some(parse_feature(&content, path))
        })
        .collect();

    BddProject {
        root_dir: root_dir.to_path_buf(),
        features,
    }
}

fn collect_feature_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_feature_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "feature") {
            out.push(path);
        }
    }
}

// ── Helper functions ─────────────────────────────────────────────────────────

fn skip_blank_and_comments(lines: &[&str], mut idx: usize) -> usize {
    while idx < lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            idx += 1;
        } else {
            break;
        }
    }
    idx
}

fn collect_tags(lines: &[&str], idx: &mut usize) -> Vec<String> {
    let mut tags = Vec::new();
    while *idx < lines.len() {
        let trimmed = lines[*idx].trim();
        if trimmed.starts_with('@') {
            for token in trimmed.split_whitespace() {
                if token.starts_with('@') {
                    tags.push(token.to_string());
                }
            }
            *idx += 1;
        } else {
            break;
        }
    }
    tags
}

fn is_structural_keyword(trimmed: &str) -> bool {
    trimmed.starts_with("Feature:")
        || trimmed.starts_with("Background:")
        || trimmed.starts_with("Scenario Outline:")
        || trimmed.starts_with("Scenario:")
        || trimmed.starts_with("Examples:")
        || is_step_line(trimmed)
}

fn is_step_line(trimmed: &str) -> bool {
    for kw in STEP_KEYWORDS {
        if trimmed.starts_with(kw) {
            let rest = &trimmed[kw.len()..];
            if rest.is_empty() || rest.starts_with(' ') {
                return true;
            }
        }
    }
    false
}

fn parse_steps(lines: &[&str], idx: &mut usize) -> Vec<BddStep> {
    let mut steps = Vec::new();
    while *idx < lines.len() {
        let trimmed = lines[*idx].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            *idx += 1;
            continue;
        }
        if let Some(step) = try_parse_step(trimmed, *idx + 1) {
            steps.push(step);
            *idx += 1;
            // Skip doc strings and data tables attached to this step
            skip_doc_string_and_table(lines, idx);
        } else {
            break;
        }
    }
    steps
}

fn try_parse_step(trimmed: &str, line_number: usize) -> Option<BddStep> {
    for kw in STEP_KEYWORDS {
        if trimmed.starts_with(kw) {
            let rest = &trimmed[kw.len()..];
            if rest.is_empty() || rest.starts_with(' ') {
                let text = rest.strip_prefix(' ').unwrap_or(rest).to_string();
                return Some(BddStep {
                    keyword: kw.to_string(),
                    text,
                    line_number,
                });
            }
        }
    }
    None
}

fn skip_doc_string_and_table(lines: &[&str], idx: &mut usize) {
    // Skip blank lines
    while *idx < lines.len() && lines[*idx].trim().is_empty() {
        *idx += 1;
    }
    if *idx >= lines.len() {
        return;
    }
    let trimmed = lines[*idx].trim();
    // Doc string
    if trimmed.starts_with("\"\"\"") || trimmed.starts_with("```") {
        let marker = &trimmed[..3];
        *idx += 1;
        while *idx < lines.len() {
            if lines[*idx].trim().starts_with(marker) {
                *idx += 1;
                break;
            }
            *idx += 1;
        }
        return;
    }
    // Data table
    if trimmed.starts_with('|') {
        while *idx < lines.len() && lines[*idx].trim().starts_with('|') {
            *idx += 1;
        }
    }
}

fn parse_examples_blocks(lines: &[&str], idx: &mut usize) -> Vec<ExamplesTable> {
    let mut examples = Vec::new();
    loop {
        // Skip blanks and comments
        let saved = *idx;
        *idx = skip_blank_and_comments(lines, *idx);

        // Collect optional tags before Examples:
        let peek = *idx;
        let tags = collect_tags(lines, idx);

        if *idx >= lines.len() {
            // If we consumed tags but didn't find Examples, revert
            if !tags.is_empty() {
                *idx = peek;
            }
            break;
        }

        let trimmed = lines[*idx].trim();
        if !trimmed.starts_with("Examples:") {
            // Revert: these tags belong to the next scenario
            *idx = if tags.is_empty() { saved } else { peek };
            break;
        }

        let ex_line = *idx + 1;
        *idx += 1;

        // Parse table rows
        let mut headers = Vec::new();
        let mut rows = Vec::new();

        // Skip blanks
        while *idx < lines.len() && lines[*idx].trim().is_empty() {
            *idx += 1;
        }

        // Header row
        if *idx < lines.len() && lines[*idx].trim().starts_with('|') {
            headers = parse_table_row(lines[*idx]);
            *idx += 1;
        }

        // Data rows
        while *idx < lines.len() && lines[*idx].trim().starts_with('|') {
            rows.push(parse_table_row(lines[*idx]));
            *idx += 1;
        }

        examples.push(ExamplesTable {
            tags,
            headers,
            rows,
            line_number: ex_line,
        });
    }
    examples
}

fn parse_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let inner = trimmed
        .strip_prefix('|')
        .unwrap_or(trimmed)
        .strip_suffix('|')
        .unwrap_or(trimmed);
    inner.split('|').map(|cell| cell.trim().to_string()).collect()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
@auth @smoke
Feature: User login
  As a registered user
  I want to sign in

  Background:
    Given I am on the login page

  @happy
  Scenario: Successful login
    When I enter username \"alice\"
    And I enter password \"correct\"
    Then I should see \"Welcome\"

  Scenario Outline: Failed login
    When I enter username \"<user>\"
    And I enter password \"<pass>\"
    Then I should see \"Error\"

    Examples:
      | user  | pass  |
      | bob   | wrong |
      | carol | bad   |
";

    #[test]
    fn parses_feature_name_and_tags() {
        let f = parse_feature(SAMPLE, PathBuf::from("login.feature"));
        assert_eq!(f.name, "User login");
        assert_eq!(f.tags, vec!["@auth", "@smoke"]);
    }

    #[test]
    fn parses_feature_description() {
        let f = parse_feature(SAMPLE, PathBuf::from("login.feature"));
        assert_eq!(f.description.len(), 2);
        assert!(f.description[0].contains("registered user"));
    }

    #[test]
    fn parses_background() {
        let f = parse_feature(SAMPLE, PathBuf::from("login.feature"));
        let bg = f.background.as_ref().expect("background should exist");
        assert_eq!(bg.steps.len(), 1);
        assert_eq!(bg.steps[0].keyword, "Given");
        assert_eq!(bg.steps[0].text, "I am on the login page");
    }

    #[test]
    fn parses_scenarios() {
        let f = parse_feature(SAMPLE, PathBuf::from("login.feature"));
        assert_eq!(f.scenarios.len(), 2);
        assert_eq!(f.scenarios[0].name, "Successful login");
        assert_eq!(f.scenarios[0].kind, ScenarioKind::Scenario);
        assert_eq!(f.scenarios[0].tags, vec!["@happy"]);
        assert_eq!(f.scenarios[0].steps.len(), 3);
        assert_eq!(f.scenarios[1].name, "Failed login");
        assert_eq!(f.scenarios[1].kind, ScenarioKind::ScenarioOutline);
    }

    #[test]
    fn parses_steps_with_keyword_and_body() {
        let f = parse_feature(SAMPLE, PathBuf::from("login.feature"));
        let s = &f.scenarios[0].steps[0];
        assert_eq!(s.keyword, "When");
        assert_eq!(s.text, "I enter username \"alice\"");
        assert!(s.line_number > 0);
    }

    #[test]
    fn parses_examples_table() {
        let f = parse_feature(SAMPLE, PathBuf::from("login.feature"));
        assert_eq!(f.scenarios[1].examples.len(), 1);
        let ex = &f.scenarios[1].examples[0];
        assert_eq!(ex.headers, vec!["user", "pass"]);
        assert_eq!(ex.rows.len(), 2);
        assert_eq!(ex.rows[0], vec!["bob", "wrong"]);
    }

    #[test]
    fn empty_content_produces_empty_feature() {
        let f = parse_feature("", PathBuf::from("empty.feature"));
        assert!(f.name.is_empty());
        assert!(f.scenarios.is_empty());
        assert!(f.background.is_none());
    }

    #[test]
    fn line_numbers_are_1_based() {
        let f = parse_feature(SAMPLE, PathBuf::from("login.feature"));
        assert!(f.background.as_ref().unwrap().line_number >= 1);
        for sc in &f.scenarios {
            assert!(sc.line_number >= 1);
            for step in &sc.steps {
                assert!(step.line_number >= 1);
            }
        }
    }
}
