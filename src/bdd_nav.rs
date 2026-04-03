//! BDD/Gherkin navigation helpers: which buffer lines are structural nodes and UTF-8 character
//! ranges for keyword tokens vs step bodies (used for TUI focus and highlighting).
use std::ops::Range;

use crate::editor_buffer::EditorBuffer;

/// Gherkin step keywords in **picker cycle** order.
pub const STEP_KEYWORDS_CYCLE: &[&str] = &["Given", "When", "Then", "And", "But"];

/// Header prefixes for navigation nodes; longer prefixes must appear before shorter ones that share a prefix.
const HEADER_PREFIXES: &[&str] = &[
    "Scenario Outline:",
    "Feature:",
    "Background:",
    "Scenario:",
    "Examples:",
];

/// Headers whose text after the colon can be focused and edited (`Feature`, `Scenario`, etc.).
///
/// `Background:` is intentionally omitted: only the keyword is navigable on that line.
const HEADER_TITLE_EDIT_PREFIXES: &[&str] =
    &["Scenario Outline:", "Feature:", "Scenario:", "Examples:"];

/// Collects row indices (ascending) for BDD navigation: `Feature:` prose lines, headers, and steps.
pub fn bdd_node_rows(buffer: &EditorBuffer) -> Vec<usize> {
    let narr = feature_narrative_row_flags(buffer);
    let mut out = Vec::new();
    for (row, &is_narr) in narr.iter().enumerate() {
        if is_narr {
            out.push(row);
            continue;
        }
        let line = buffer.line(row);
        if line_is_bdd_node(&line) {
            out.push(row);
        }
    }
    out
}

/// Row indices in document order for **body** vertical navigation: steps plus editable header titles
/// (`Feature:` / `Scenario:` / `Scenario Outline:` / `Examples:`), so `↑`/`↓` can move between steps
/// and the scenario/feature title lines.
pub fn bdd_step_and_header_title_rows(buffer: &EditorBuffer) -> Vec<usize> {
    let mut out = Vec::new();
    for row in 0..buffer.line_count() {
        let line = buffer.line(row);
        if step_edit_start_col(&line).is_some() || header_title_edit_start_col(&line).is_some() {
            out.push(row);
        }
    }
    out
}

/// Returns the smallest node row strictly greater than `current_row`, if any.
pub fn next_node_row(rows: &[usize], current_row: usize) -> Option<usize> {
    rows.iter().find(|&&r| r > current_row).copied()
}

/// Returns the greatest node row strictly less than `current_row`, if any.
pub fn prev_node_row(rows: &[usize], current_row: usize) -> Option<usize> {
    let mut best: Option<usize> = None;
    for &r in rows {
        if r < current_row {
            best = Some(r);
        }
    }
    best
}

fn line_is_bdd_node(line: &str) -> bool {
    let trimmed = line.trim_start();
    HEADER_PREFIXES.iter().any(|p| trimmed.starts_with(p))
        || step_keyword_at_line_start(trimmed).is_some()
}

/// Ends the free-text block after `Feature:`: the next structural header or a step line.
fn feature_description_terminator(trimmed: &str) -> bool {
    trimmed.starts_with("Scenario Outline:")
        || trimmed.starts_with("Background:")
        || trimmed.starts_with("Scenario:")
        || trimmed.starts_with("Examples:")
        || step_keyword_at_line_start(trimmed).is_some()
}

/// One entry per buffer row: line is feature prose (between `Feature:` and the next structural block).
pub fn feature_narrative_row_flags(buffer: &EditorBuffer) -> Vec<bool> {
    let n = buffer.line_count();
    let mut flags = vec![false; n];
    let mut in_feature_description = false;
    for (row, flag) in flags.iter_mut().enumerate() {
        let line = buffer.line(row);
        let trimmed = line.trim_start();
        if trimmed.starts_with("Feature:") {
            in_feature_description = true;
            continue;
        }
        if in_feature_description {
            if feature_description_terminator(trimmed) {
                in_feature_description = false;
            } else {
                *flag = true;
            }
        }
    }
    flags
}

/// `true` when `row` is navigable and editable feature description text (not the `Feature:` header line).
pub fn is_feature_narrative_row(buffer: &EditorBuffer, row: usize) -> bool {
    row < buffer.line_count() && feature_narrative_row_flags(buffer)[row]
}

fn step_keyword_at_line_start(trimmed: &str) -> Option<&'static str> {
    for kw in STEP_KEYWORDS_CYCLE {
        if trimmed.strip_prefix(kw).is_some() {
            return Some(*kw);
        }
    }
    None
}

/// UTF-8 character index range for the leading Gherkin keyword token (header or step keyword).
///
/// Indices count Unicode scalar values (Rust `char`), matching the editor buffer column model.
pub fn keyword_char_range(line: &str) -> Option<Range<usize>> {
    let trimmed = line.trim_start();
    let leading = line.len().saturating_sub(trimmed.len());
    for p in HEADER_PREFIXES {
        if let Some(rest) = trimmed.strip_prefix(*p) {
            let _ = rest;
            let end = leading + p.chars().count();
            return Some(leading..end);
        }
    }
    if let Some(kw) = step_keyword_at_line_start(trimmed) {
        let end = leading + kw.chars().count();
        return Some(leading..end);
    }
    None
}

/// UTF-8 character index range for editable step body text (after keyword and one optional space).
///
/// Returns `None` when the line is not a step line. The range may be empty (`start == end`) when the body is empty.
pub fn body_char_range(line: &str) -> Option<Range<usize>> {
    let body_start = step_edit_start_col(line)?;
    let end = line.chars().count();
    Some(body_start..end)
}

/// First UTF-8 column where the header title starts (after `Feature:` / `Scenario:` / … and one optional space).
///
/// Applies only to [`HEADER_TITLE_EDIT_PREFIXES`], not `Background:`.
pub fn header_title_edit_start_col(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let leading = line.len().saturating_sub(trimmed.len());
    for p in HEADER_TITLE_EDIT_PREFIXES {
        if let Some(rest) = trimmed.strip_prefix(*p) {
            let mut col = leading + p.chars().count();
            if rest.starts_with(' ') {
                col += 1;
            }
            return Some(col);
        }
    }
    None
}

/// First editable UTF-8 column for the navigable "body": step text or a supported header title.
pub fn line_body_edit_min_col(line: &str) -> Option<usize> {
    step_edit_start_col(line).or_else(|| header_title_edit_start_col(line))
}

/// Like [`line_body_edit_min_col`], including feature description rows (whole line from column `0`).
pub fn line_body_edit_min_col_in_buffer(buffer: &EditorBuffer, row: usize) -> Option<usize> {
    if is_feature_narrative_row(buffer, row) {
        return Some(0);
    }
    line_body_edit_min_col(&buffer.line(row))
}

/// UTF-8 range highlighted for body/title focus in the editor (step line or editable header).
pub fn nav_body_char_range(line: &str) -> Option<Range<usize>> {
    let end = line.chars().count();
    line_body_edit_min_col(line).map(|s| s..end)
}

/// Full-line body range for feature narrative rows; otherwise same as [`nav_body_char_range`].
pub fn nav_body_char_range_in_buffer(
    buffer: &EditorBuffer,
    row: usize,
    line: &str,
) -> Option<Range<usize>> {
    let end = line.chars().count();
    if is_feature_narrative_row(buffer, row) {
        return Some(0..end);
    }
    nav_body_char_range(line)
}

/// Returns the first UTF-8 character column where step body text starts, or `None` if not a step line.
pub fn step_edit_start_col(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let leading = line.len().saturating_sub(trimmed.len());
    for kw in STEP_KEYWORDS_CYCLE {
        if let Some(rest) = trimmed.strip_prefix(*kw) {
            let mut col = leading + kw.chars().count();
            if rest.starts_with(' ') {
                col += 1;
            }
            return Some(col);
        }
    }
    None
}

/// Returns the index into [`STEP_KEYWORDS_CYCLE`] for the leading step keyword, if any.
pub fn current_step_keyword_index(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    for (i, kw) in STEP_KEYWORDS_CYCLE.iter().enumerate() {
        if trimmed.strip_prefix(*kw).is_some() {
            return Some(i);
        }
    }
    None
}

/// Replaces the leading step keyword with `new_keyword`, preserving indentation and the rest of the line.
///
/// Returns `None` if `new_keyword` is not a known step keyword or the line does not start with one.
pub fn replace_step_keyword_line(line: &str, new_keyword: &str) -> Option<String> {
    if !STEP_KEYWORDS_CYCLE.contains(&new_keyword) {
        return None;
    }
    let trimmed = line.trim_start();
    let leading = line.len().saturating_sub(trimmed.len());
    let leading_s = line.get(..leading).unwrap_or("");
    for kw in STEP_KEYWORDS_CYCLE {
        if let Some(rest) = trimmed.strip_prefix(*kw) {
            let new_trimmed = format!("{new_keyword}{rest}");
            return Some(format!("{leading_s}{new_trimmed}"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_buffer::EditorBuffer;

    #[test]
    fn test_bdd_node_rows_headers_and_steps() {
        let buf = EditorBuffer::from_string(
            "@t\nFeature: A\n  Scenario: S\n  Given x\n  Examples:\n".to_string(),
        );
        let rows = bdd_node_rows(&buf);
        assert_eq!(rows, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_feature_narrative_rows_in_bdd_nodes() {
        let buf = EditorBuffer::from_string(
            "Feature: T\n  One\n  Two\nBackground:\n  Given x\n".to_string(),
        );
        let flags = feature_narrative_row_flags(&buf);
        assert!(!flags[0]);
        assert!(flags[1] && flags[2]);
        assert!(!flags[3]);
        assert_eq!(bdd_node_rows(&buf), vec![0, 1, 2, 3, 4]);
        assert_eq!(line_body_edit_min_col_in_buffer(&buf, 1), Some(0));
    }

    #[test]
    fn test_bdd_step_rows_only_steps() {
        let buf = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n  Given a\n  Scenario: T\n  When b\n".to_string(),
        );
        assert_eq!(bdd_node_rows(&buf), vec![0, 1, 2, 3, 4]);
        let step_only_rows: Vec<usize> = (0..buf.line_count())
            .filter(|&r| step_edit_start_col(&buf.line(r)).is_some())
            .collect();
        assert_eq!(step_only_rows, vec![2, 4]);
    }

    #[test]
    fn test_bdd_step_and_header_title_rows_merges_document_order() {
        let buf = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n  Given a\n  Scenario: T\n  When b\n".to_string(),
        );
        assert_eq!(bdd_step_and_header_title_rows(&buf), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn test_next_prev_node_row() {
        let rows = vec![1, 3, 5];
        assert_eq!(next_node_row(&rows, 0), Some(1));
        assert_eq!(next_node_row(&rows, 1), Some(3));
        assert_eq!(next_node_row(&rows, 5), None);
        assert_eq!(prev_node_row(&rows, 6), Some(5));
        assert_eq!(prev_node_row(&rows, 5), Some(3));
        assert_eq!(prev_node_row(&rows, 0), None);
    }

    #[test]
    fn test_keyword_char_range_header_and_step() {
        assert_eq!(keyword_char_range("  Feature: A"), Some(2..10));
        assert_eq!(keyword_char_range("    Scenario Outline: X"), Some(4..21));
        assert_eq!(keyword_char_range("  Given hello"), Some(2..7));
        assert_eq!(keyword_char_range("When x"), Some(0..4));
    }

    #[test]
    fn test_body_char_range() {
        assert_eq!(body_char_range("  Given I log in"), Some(8..16));
        assert_eq!(body_char_range("Given"), Some(5..5));
        assert_eq!(body_char_range("Feature: x"), None);
    }

    #[test]
    fn test_header_title_edit_and_nav_body() {
        assert_eq!(header_title_edit_start_col("  Feature: My feat"), Some(11));
        assert_eq!(header_title_edit_start_col("Scenario: S"), Some(10));
        assert_eq!(
            header_title_edit_start_col("  Scenario Outline: SO"),
            Some(20)
        );
        assert_eq!(header_title_edit_start_col("  Examples:"), Some(11));
        assert_eq!(header_title_edit_start_col("  Background: B"), None);
        assert_eq!(nav_body_char_range("  Feature: X"), Some(11..12));
        assert_eq!(line_body_edit_min_col("  When x"), Some(7));
    }

    #[test]
    fn test_step_edit_start_col() {
        assert_eq!(step_edit_start_col("  Given I log in"), Some(8));
        assert_eq!(step_edit_start_col("When x"), Some(5));
        assert_eq!(step_edit_start_col("Feature: x"), None);
    }
}
