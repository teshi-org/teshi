//! Simple line-diff algorithm for computing visual diffs of Gherkin buffers.
//!
//! Implements a longest-common-subsequence (LCS) based line diff, producing
//! [`DiffLine`] entries annotated with [`ChangeKind`] for UI rendering.

use std::collections::HashMap;

/// The kind of change a line represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Unchanged,
}

/// One line of a computed diff between old and new buffer content.
#[derive(Debug, Clone)]
pub struct DiffLine {
    /// 1-based line number in the *new* buffer (0 for deleted lines).
    pub line_number_1based: usize,
    pub kind: ChangeKind,
    /// The text in the new buffer (or the old text for deleted lines).
    pub text: String,
    /// The old text for Modified/Deleted lines.
    pub old_text: Option<String>,
}

/// Compute a line-diff between old and new content using LCS.
///
/// Returns a list of `DiffLine`s that can be used to render a visual diff.
/// Lines present in both buffers are `Unchanged`, lines only in the new
/// buffer are `Added`, lines only in the old buffer are `Deleted`, and
/// lines whose content changed at the same position are `Modified`.
pub fn diff_buffers(old_content: &str, new_content: &str) -> Vec<DiffLine> {
    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();

    let lcs = lcs_indices(&old_lines, &new_lines);
    let mut old_idx = 0usize;
    let mut new_idx = 0usize;
    let mut result: Vec<DiffLine> = Vec::new();

    for &(lcs_old, lcs_new) in &lcs {
        // Lines in old but not in new → deleted
        while old_idx < lcs_old {
            result.push(DiffLine {
                line_number_1based: old_idx + 1,
                kind: ChangeKind::Deleted,
                text: old_lines[old_idx].to_string(),
                old_text: Some(old_lines[old_idx].to_string()),
            });
            old_idx += 1;
        }
        // Lines in new but not in old → added
        while new_idx < lcs_new {
            result.push(DiffLine {
                line_number_1based: new_idx + 1,
                kind: ChangeKind::Added,
                text: new_lines[new_idx].to_string(),
                old_text: None,
            });
            new_idx += 1;
        }
        // LCS match — check if modified or unchanged
        let old_text = old_lines[lcs_old];
        let new_text = new_lines[lcs_new];
        if old_text == new_text {
            result.push(DiffLine {
                line_number_1based: new_idx + 1,
                kind: ChangeKind::Unchanged,
                text: new_text.to_string(),
                old_text: None,
            });
        } else {
            result.push(DiffLine {
                line_number_1based: new_idx + 1,
                kind: ChangeKind::Modified,
                text: new_text.to_string(),
                old_text: Some(old_text.to_string()),
            });
        }
        old_idx = lcs_old + 1;
        new_idx = lcs_new + 1;
    }

    // Remaining old lines → deleted
    while old_idx < old_lines.len() {
        result.push(DiffLine {
            line_number_1based: old_idx + 1,
            kind: ChangeKind::Deleted,
            text: old_lines[old_idx].to_string(),
            old_text: Some(old_lines[old_idx].to_string()),
        });
        old_idx += 1;
    }
    // Remaining new lines → added
    while new_idx < new_lines.len() {
        result.push(DiffLine {
            line_number_1based: new_idx + 1,
            kind: ChangeKind::Added,
            text: new_lines[new_idx].to_string(),
            old_text: None,
        });
        new_idx += 1;
    }

    // Post-process: merge adjacent Deleted+Added into Modified
    result = merge_adjacent_deleted_added(result);

    result
}

/// Merge adjacent Deleted+Added pairs into single Modified entries.
///
/// When a line is changed (rather than truly removed and a new one inserted),
/// the LCS produces a Deleted immediately followed by an Added. This function
/// merges those pairs into one [`DiffLine`] with [`ChangeKind::Modified`].
fn merge_adjacent_deleted_added(lines: Vec<DiffLine>) -> Vec<DiffLine> {
    let mut merged: Vec<DiffLine> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if i + 1 < lines.len()
            && lines[i].kind == ChangeKind::Deleted
            && lines[i + 1].kind == ChangeKind::Added
        {
            merged.push(DiffLine {
                line_number_1based: lines[i + 1].line_number_1based,
                kind: ChangeKind::Modified,
                text: lines[i + 1].text.clone(),
                old_text: Some(lines[i].text.clone()),
            });
            i += 2;
        } else {
            merged.push(lines[i].clone());
            i += 1;
        }
    }
    merged
}

/// Compute LCS indices as `(old_idx, new_idx)` pairs.
///
/// Uses a hash-map based approach: group old lines by content, walk new lines
/// and match them greedily to produce the LCS alignment.
fn lcs_indices(old: &[&str], new: &[&str]) -> Vec<(usize, usize)> {
    // Build position list for each unique line in old
    let mut old_positions: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, line) in old.iter().enumerate() {
        old_positions.entry(line).or_default().push(i);
    }

    // For each new line, consume one matching old position (if any)
    let mut old_used = vec![false; old.len()];
    let mut lcs: Vec<(usize, usize)> = Vec::new();
    let mut last_old: usize = 0;

    for (new_idx, line) in new.iter().enumerate() {
        if let Some(positions) = old_positions.get(line) {
            // Find the first unused position >= last_old (maintains order)
            for &old_idx in positions {
                if !old_used[old_idx] && old_idx >= last_old {
                    old_used[old_idx] = true;
                    lcs.push((old_idx, new_idx));
                    last_old = old_idx + 1;
                    break;
                }
            }
        }
    }

    lcs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_content() {
        let content = "Feature: Test\n  Scenario: S1\n    Given a step";
        let result = diff_buffers(content, content);
        assert!(result.iter().all(|d| d.kind == ChangeKind::Unchanged));
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_insert_line() {
        let old = "Line1\nLine2\nLine3";
        let new = "Line1\nLine2\nNewLine\nLine3";
        let result = diff_buffers(old, new);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].kind, ChangeKind::Unchanged);
        assert_eq!(result[1].kind, ChangeKind::Unchanged);
        assert_eq!(result[2].kind, ChangeKind::Added);
        assert_eq!(result[2].text, "NewLine");
        assert_eq!(result[3].kind, ChangeKind::Unchanged);
    }

    #[test]
    fn test_delete_line() {
        let old = "Line1\nLine2\nLine3";
        let new = "Line1\nLine3";
        let result = diff_buffers(old, new);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].kind, ChangeKind::Unchanged);
        assert_eq!(result[1].kind, ChangeKind::Deleted);
        assert_eq!(result[2].kind, ChangeKind::Unchanged);
    }

    #[test]
    fn test_modify_line() {
        let old = "Feature: Old name";
        let new = "Feature: New name";
        let result = diff_buffers(old, new);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, ChangeKind::Modified);
    }

    #[test]
    fn test_insert_multiline_block() {
        let old = "Feature: F\n  Scenario: S1\n    Given a";
        let new =
            "Feature: F\n  Scenario: S1\n    Given a\n\n  Scenario: S2\n    Given b\n    When c";
        let result = diff_buffers(old, new);
        assert!(
            result.len() >= 4,
            "should have at least 4 diff lines, got {}",
            result.len()
        );
        let added: Vec<&DiffLine> = result
            .iter()
            .filter(|d| d.kind == ChangeKind::Added)
            .collect();
        assert_eq!(
            added.len(),
            4,
            "expected 4 Added lines (blank + header + 2 steps)"
        );
    }
}
