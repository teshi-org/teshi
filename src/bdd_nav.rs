//! BDD/Gherkin navigation helpers for step-focused editor navigation and editing ranges.
use std::ops::Range;

use crate::editor_buffer::EditorBuffer;
use crate::gherkin_keywords::HEADER_TITLE_EDIT_PREFIXES;

/// Gherkin step keywords in **picker cycle** order (re-exported from shared module).
pub use crate::gherkin_keywords::STEP_KEYWORDS as STEP_KEYWORDS_CYCLE;

/// Collects row indices (ascending) for step-only navigation.
pub fn bdd_step_rows(buffer: &EditorBuffer) -> Vec<usize> {
    let mut out = Vec::new();
    for row in 0..buffer.line_count() {
        if step_edit_start_col(&buffer.line(row)).is_some() {
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

fn leading_whitespace(line: &str) -> String {
    line.chars().take_while(|ch| ch.is_whitespace()).collect()
}

fn is_feature_header(trimmed: &str) -> bool {
    trimmed.starts_with("Feature:")
}

fn is_scenario_header(trimmed: &str) -> bool {
    trimmed.starts_with("Scenario:") || trimmed.starts_with("Scenario Outline:")
}

fn is_scenario_boundary(trimmed: &str) -> bool {
    is_feature_header(trimmed) || trimmed.starts_with("Background:") || is_scenario_header(trimmed)
}

fn scenario_block_end(buffer: &EditorBuffer, scenario_row: usize) -> usize {
    let mut row = scenario_row + 1;
    while row < buffer.line_count() {
        let line = buffer.line(row);
        if row + 1 == buffer.line_count() && line.is_empty() {
            break;
        }
        if is_scenario_boundary(line.trim_start()) {
            break;
        }
        row += 1;
    }
    row
}

fn step_block_end(buffer: &EditorBuffer, step_row: usize) -> usize {
    let scenario_row = scenario_header_for_row(buffer, step_row).unwrap_or(step_row);
    let scenario_end = scenario_block_end(buffer, scenario_row);
    let mut row = step_row + 1;
    while row < scenario_end {
        let line = buffer.line(row);
        let trimmed = line.trim_start();
        if step_edit_start_col(&line).is_some() || is_scenario_boundary(trimmed) {
            break;
        }
        row += 1;
    }
    row
}

fn line_vec(buffer: &EditorBuffer) -> (Vec<String>, bool) {
    let text = buffer.as_string();
    let trailing_newline = text.ends_with('\n');
    let mut lines = (0..buffer.line_count())
        .map(|row| buffer.line(row))
        .collect::<Vec<_>>();
    if trailing_newline && lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    (lines, trailing_newline)
}

fn write_line_vec(buffer: &mut EditorBuffer, lines: Vec<String>, trailing_newline: bool) {
    let normalized = if lines.is_empty() {
        String::new()
    } else {
        let mut text = lines.join("\n");
        if trailing_newline {
            text.push('\n');
        }
        text
    };
    *buffer = EditorBuffer::from_string(normalized);
}

fn insert_lines(lines: &mut Vec<String>, at: usize, new_lines: &[String]) {
    lines.splice(at..at, new_lines.iter().cloned());
}

fn remove_range(lines: &mut Vec<String>, range: Range<usize>) {
    lines.drain(range);
    if lines.is_empty() {
        lines.push(String::new());
    }
}

pub fn step_block_lines(buffer: &EditorBuffer, step_row: usize) -> Option<Vec<String>> {
    let start = step_row.min(buffer.line_count().saturating_sub(1));
    step_edit_start_col(&buffer.line(start))?;
    let end = step_block_end(buffer, start);
    Some((start..end).map(|row| buffer.line(row)).collect())
}

fn default_step_line(buffer: &EditorBuffer, scenario_row: usize) -> String {
    if let Some(first_step_row) = scenario_step_rows(buffer, scenario_row).first().copied() {
        let line = buffer.line(first_step_row);
        let start = step_edit_start_col(&line).unwrap_or_else(|| line.chars().count());
        let prefix: String = line.chars().take(start).collect();
        return prefix;
    }
    let line = buffer.line(scenario_row);
    let indent = leading_whitespace(&line);
    format!("{indent}  Given ")
}

/// Returns the `Scenario:` / `Scenario Outline:` row that owns `row`, if any.
pub fn scenario_header_for_row(buffer: &EditorBuffer, row: usize) -> Option<usize> {
    if row >= buffer.line_count() {
        return None;
    }
    for candidate in (0..=row).rev() {
        let line = buffer.line(candidate);
        let trimmed = line.trim_start();
        if is_scenario_header(trimmed) {
            return Some(candidate);
        }
        if candidate != row && (is_feature_header(trimmed) || trimmed.starts_with("Background:")) {
            break;
        }
    }
    None
}

/// Returns step-header rows in the current scenario block.
pub fn scenario_step_rows(buffer: &EditorBuffer, scenario_row: usize) -> Vec<usize> {
    if scenario_row >= buffer.line_count() {
        return Vec::new();
    }
    let line = buffer.line(scenario_row);
    if !is_scenario_header(line.trim_start()) {
        return Vec::new();
    }
    let mut rows = Vec::new();
    let end = scenario_block_end(buffer, scenario_row);
    for row in (scenario_row + 1)..end {
        let line = buffer.line(row);
        if step_edit_start_col(&line).is_some() {
            rows.push(row);
        }
    }
    rows
}

/// Returns all non-header rows inside a scenario block.
pub fn scenario_content_rows(buffer: &EditorBuffer, scenario_row: usize) -> Vec<usize> {
    if scenario_row >= buffer.line_count() {
        return Vec::new();
    }
    let line = buffer.line(scenario_row);
    if !is_scenario_header(line.trim_start()) {
        return Vec::new();
    }
    ((scenario_row + 1)..scenario_block_end(buffer, scenario_row)).collect()
}

/// Inserts a new step after the current step (or directly under the current scenario header).
pub fn insert_step_below(buffer: &mut EditorBuffer, row: usize) -> Option<usize> {
    let scenario_row = scenario_header_for_row(buffer, row)?;
    let new_line = default_step_line(buffer, scenario_row);
    let insert_at = if step_edit_start_col(&buffer.line(row)).is_some() {
        step_block_end(buffer, row)
    } else {
        scenario_row + 1
    };
    let (mut lines, trailing_newline) = line_vec(buffer);
    insert_lines(&mut lines, insert_at, &[new_line]);
    write_line_vec(buffer, lines, trailing_newline);
    Some(insert_at)
}

/// Inserts a new step before the current step (or as the first step in the current scenario).
pub fn insert_step_above(buffer: &mut EditorBuffer, row: usize) -> Option<usize> {
    let scenario_row = scenario_header_for_row(buffer, row)?;
    let new_line = default_step_line(buffer, scenario_row);
    let insert_at = if step_edit_start_col(&buffer.line(row)).is_some() {
        row
    } else {
        scenario_row + 1
    };
    let (mut lines, trailing_newline) = line_vec(buffer);
    insert_lines(&mut lines, insert_at, &[new_line]);
    write_line_vec(buffer, lines, trailing_newline);
    Some(insert_at)
}

/// Inserts a new scenario header after the current scenario block.
pub fn insert_scenario_after_current(buffer: &mut EditorBuffer, row: usize) -> Option<usize> {
    let scenario_row = scenario_header_for_row(buffer, row)?;
    let line = buffer.line(scenario_row);
    let indent = leading_whitespace(&line);
    let insert_at = scenario_block_end(buffer, scenario_row);
    let new_line = format!("{indent}Scenario: ");
    let (mut lines, trailing_newline) = line_vec(buffer);
    insert_lines(&mut lines, insert_at, &[new_line]);
    write_line_vec(buffer, lines, trailing_newline);
    Some(insert_at)
}

/// Deletes the full current step block, including attached doc-string / table rows.
pub fn delete_step(buffer: &mut EditorBuffer, row: usize) -> Option<usize> {
    let line = buffer.line(row);
    step_edit_start_col(&line)?;
    let start = row;
    let end = step_block_end(buffer, row);
    let (mut lines, trailing_newline) = line_vec(buffer);
    remove_range(&mut lines, start..end);
    write_line_vec(buffer, lines, trailing_newline);
    Some(start.saturating_sub(1))
}

/// Deletes the full current scenario block, including all steps and examples.
pub fn delete_scenario_block(buffer: &mut EditorBuffer, scenario_row: usize) -> Option<usize> {
    let line = buffer.line(scenario_row);
    if !is_scenario_header(line.trim_start()) {
        return None;
    }
    let end = scenario_block_end(buffer, scenario_row);
    let (mut lines, trailing_newline) = line_vec(buffer);
    remove_range(&mut lines, scenario_row..end);
    write_line_vec(buffer, lines, trailing_newline);
    Some(scenario_row.saturating_sub(1))
}

/// Swaps the current full step block with the previous step block in the same scenario.
pub fn swap_step_with_prev(buffer: &mut EditorBuffer, row: usize) -> Option<usize> {
    let scenario_row = scenario_header_for_row(buffer, row)?;
    step_edit_start_col(&buffer.line(row))?;
    let steps = scenario_step_rows(buffer, scenario_row);
    let index = steps.iter().position(|&step_row| step_row == row)?;
    if index == 0 {
        return None;
    }
    let prev_row = steps[index - 1];
    let prev_end = step_block_end(buffer, prev_row);
    let current_end = step_block_end(buffer, row);
    let (mut lines, trailing_newline) = line_vec(buffer);
    let prev_block: Vec<String> = lines[prev_row..prev_end].to_vec();
    let current_block: Vec<String> = lines[row..current_end].to_vec();
    lines.splice(
        prev_row..current_end,
        current_block.into_iter().chain(prev_block),
    );
    write_line_vec(buffer, lines, trailing_newline);
    Some(prev_row)
}

/// Swaps the current full step block with the next step block in the same scenario.
pub fn swap_step_with_next(buffer: &mut EditorBuffer, row: usize) -> Option<usize> {
    let scenario_row = scenario_header_for_row(buffer, row)?;
    step_edit_start_col(&buffer.line(row))?;
    let steps = scenario_step_rows(buffer, scenario_row);
    let index = steps.iter().position(|&step_row| step_row == row)?;
    let next_row = *steps.get(index + 1)?;
    let current_end = step_block_end(buffer, row);
    let next_end = step_block_end(buffer, next_row);
    let (mut lines, trailing_newline) = line_vec(buffer);
    let current_block: Vec<String> = lines[row..current_end].to_vec();
    let next_block: Vec<String> = lines[next_row..next_end].to_vec();
    let next_block_len = next_block.len();
    lines.splice(row..next_end, next_block.into_iter().chain(current_block));
    write_line_vec(buffer, lines, trailing_newline);
    Some(row + next_block_len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_buffer::EditorBuffer;

    #[test]
    fn test_feature_narrative_rows() {
        let buf = EditorBuffer::from_string(
            "Feature: T\n  One\n  Two\nBackground:\n  Given x\n".to_string(),
        );
        let flags = feature_narrative_row_flags(&buf);
        assert!(!flags[0]);
        assert!(flags[1] && flags[2]);
        assert!(!flags[3]);
        assert_eq!(line_body_edit_min_col_in_buffer(&buf, 1), Some(0));
    }

    #[test]
    fn test_bdd_step_rows_only_steps() {
        let buf = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n  Given a\n  Scenario: T\n  When b\n".to_string(),
        );
        assert_eq!(bdd_step_rows(&buf), vec![2, 4]);
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

    #[test]
    fn test_scenario_helpers_detect_owner_and_steps() {
        let buf = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n    Given one\n    And two\n  Scenario: T\n".to_string(),
        );
        assert_eq!(scenario_header_for_row(&buf, 1), Some(1));
        assert_eq!(scenario_header_for_row(&buf, 2), Some(1));
        assert_eq!(scenario_step_rows(&buf, 1), vec![2, 3]);
        assert_eq!(scenario_content_rows(&buf, 1), vec![2, 3]);
    }

    #[test]
    fn test_insert_step_above_and_below_preserve_indent() {
        let mut buf = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n    Given one\n    Then two\n".to_string(),
        );
        let below = insert_step_below(&mut buf, 2);
        assert_eq!(below, Some(3));
        assert_eq!(buf.line(3), "    Given ");

        let above = insert_step_above(&mut buf, 4);
        assert_eq!(above, Some(4));
        assert_eq!(buf.line(4), "    Given ");
    }

    #[test]
    fn test_insert_scenario_after_current_adds_header_after_block() {
        let mut buf = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n    Given one\n    Then two\n".to_string(),
        );
        let row = insert_scenario_after_current(&mut buf, 2);
        assert_eq!(row, Some(4));
        assert_eq!(buf.line(4), "  Scenario: ");
    }

    #[test]
    fn test_delete_step_removes_attached_doc_string_block() {
        let mut buf = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n    Given one\n      \"\"\"\n      body\n      \"\"\"\n    Then two\n"
                .to_string(),
        );
        let row = delete_step(&mut buf, 2);
        assert_eq!(row, Some(1));
        assert_eq!(buf.line(2), "    Then two");
    }

    #[test]
    fn test_delete_scenario_block_removes_examples_and_steps() {
        let mut buf = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n    Given one\n    Then two\n  Scenario: T\n    When next\n"
                .to_string(),
        );
        let row = delete_scenario_block(&mut buf, 1);
        assert_eq!(row, Some(0));
        assert_eq!(buf.line(1), "  Scenario: T");
    }

    #[test]
    fn test_swap_step_moves_full_block() {
        let mut buf = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n    Given one\n      | a |\n    Then two\n".to_string(),
        );
        let down = swap_step_with_next(&mut buf, 2);
        assert_eq!(down, Some(3));
        assert_eq!(buf.line(2), "    Then two");
        assert_eq!(buf.line(3), "    Given one");
        assert_eq!(buf.line(4), "      | a |");

        let up = swap_step_with_prev(&mut buf, 3);
        assert_eq!(up, Some(2));
        assert_eq!(buf.line(2), "    Given one");
        assert_eq!(buf.line(3), "      | a |");
        assert_eq!(buf.line(4), "    Then two");
    }
}
