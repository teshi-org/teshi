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

/// Collects row indices (ascending) for BDD structural rows: Gherkin headers or step lines.
pub fn bdd_node_rows(buffer: &EditorBuffer) -> Vec<usize> {
    let mut out = Vec::new();
    for row in 0..buffer.line_count() {
        let line = buffer.line(row);
        if line_is_bdd_node(&line) {
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
    fn test_step_edit_start_col() {
        assert_eq!(step_edit_start_col("  Given I log in"), Some(8));
        assert_eq!(step_edit_start_col("When x"), Some(5));
        assert_eq!(step_edit_start_col("Feature: x"), None);
    }
}
