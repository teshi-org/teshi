//! Optional YAML frontmatter parsing and body stripping for issue/requirement Markdown.
//!
//! Frontmatter is strictly optional and delimited by `---` lines at the very
//! top of a Markdown file. Anchor positions, quotes, and document revisions are
//! computed strictly over the stripped body so editing metadata such as
//! `status: testing` never invalidates existing test-point anchors.

use serde::{Deserialize, Serialize};

use super::anchors::TextRange;

/// Normalized issue kind derived from frontmatter `type` (defaults to [`IssueKind::Feat`]).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    /// Feature or requirement document (default when `type` is omitted or not `bug`).
    #[default]
    Feat,
    /// Bug or defect re-test ticket (`type: bug`, case-insensitive).
    Bug,
}

impl IssueKind {
    /// Parses an optional frontmatter `type` value into an [`IssueKind`].
    pub fn from_frontmatter(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some(kind) if kind.eq_ignore_ascii_case("bug") => Self::Bug,
            _ => Self::Feat,
        }
    }

    /// Uppercase badge text rendered beside each issue in the list (`[FEAT]` / `[BUG]`).
    pub const fn badge(self) -> &'static str {
        match self {
            Self::Feat => "[FEAT]",
            Self::Bug => "[BUG]",
        }
    }
}

/// Optional YAML frontmatter metadata at the top of a Markdown document.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentFrontmatter {
    /// Document kind (e.g. `feat`, `bug`).
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub issue_type: Option<String>,
    /// Workflow status (e.g. `open`, `testing`, `done`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Optional display title override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Optional external reference (e.g. issue key or URL).
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<String>,
}

impl DocumentFrontmatter {
    /// Returns the normalized issue kind (`Bug` for `type: bug`, otherwise `Feat`).
    pub fn kind(&self) -> IssueKind {
        IssueKind::from_frontmatter(self.issue_type.as_deref())
    }

    /// Returns `true` when frontmatter `status` is `done` (case-insensitive).
    pub fn is_done(&self) -> bool {
        self.status
            .as_deref()
            .is_some_and(|status| status.trim().eq_ignore_ascii_case("done"))
    }
}

/// A Markdown document split into optional frontmatter and stripped body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMarkdownDocument<'a> {
    /// Parsed frontmatter when a valid YAML frontmatter block is present.
    pub frontmatter: Option<DocumentFrontmatter>,
    /// Markdown body after stripping any leading frontmatter block.
    pub body: &'a str,
    /// UTF-8 byte offset in the raw input where `body` starts (`0` when absent).
    pub body_byte_offset: usize,
    /// Unicode scalar (`char`) offset in the raw input where `body` starts (`0` when absent).
    pub body_char_offset: u32,
}

/// Parses optional YAML frontmatter from the start of `raw` and returns the
/// stripped body along with its byte and character offsets.
///
/// Only a `---` line on the first line closed by a subsequent `---` or `...`
/// line is treated as frontmatter. Unclosed `---` blocks are left untouched as
/// ordinary Markdown. Blank lines immediately between the closing delimiter and
/// the body are stripped so adding frontmatter above an existing `# Title`
/// keeps the body text, revision, and anchors byte-identical.
pub fn parse_markdown_frontmatter(raw: &str) -> ParsedMarkdownDocument<'_> {
    let Some(split) = split_frontmatter_raw(raw) else {
        return ParsedMarkdownDocument {
            frontmatter: None,
            body: raw,
            body_byte_offset: 0,
            body_char_offset: 0,
        };
    };

    let mut fm = DocumentFrontmatter::default();
    for (key, value) in iter_frontmatter_entries(split.frontmatter_block) {
        match key {
            "type" => fm.issue_type = Some(value.to_string()),
            "status" => fm.status = Some(value.to_string()),
            "title" => fm.title = Some(value.to_string()),
            "ref" => fm.external_ref = Some(value.to_string()),
            _ => {}
        }
    }

    let body_char_offset = raw[..split.body_byte_offset].chars().count() as u32;
    ParsedMarkdownDocument {
        frontmatter: Some(fm),
        body: &raw[split.body_byte_offset..],
        body_byte_offset: split.body_byte_offset,
        body_char_offset,
    }
}

/// Returns the normalized [`IssueKind`] for a raw Markdown document.
pub fn document_issue_kind(raw: &str) -> IssueKind {
    parse_markdown_frontmatter(raw)
        .frontmatter
        .map_or(IssueKind::Feat, |fm| fm.kind())
}

/// Returns `true` when a raw Markdown document's frontmatter sets `status: done`.
pub fn document_is_done(raw: &str) -> bool {
    parse_markdown_frontmatter(raw)
        .frontmatter
        .is_some_and(|fm| fm.is_done())
}

/// Returns the Markdown body with any leading YAML frontmatter stripped.
pub fn strip_markdown_frontmatter(raw: &str) -> &str {
    split_frontmatter_raw(raw)
        .map(|split| &raw[split.body_byte_offset..])
        .unwrap_or(raw)
}

/// Returns the Unicode scalar offset in `raw` where the stripped body starts.
pub fn frontmatter_body_char_offset(raw: &str) -> u32 {
    split_frontmatter_raw(raw)
        .map(|split| raw[..split.body_byte_offset].chars().count() as u32)
        .unwrap_or(0)
}

/// Extracts the borrowed `title` scalar from frontmatter when present and non-empty.
pub(crate) fn frontmatter_title_slice(raw: &str) -> Option<&str> {
    let split = split_frontmatter_raw(raw)?;
    let mut title = None;
    for (key, value) in iter_frontmatter_entries(split.frontmatter_block) {
        if key == "title" {
            title = Some(value);
        }
    }
    title
}

/// Converts a character range in raw Markdown into a stripped-body range.
///
/// Returns `None` when the range is empty or begins inside the frontmatter
/// block.
pub fn raw_range_to_body_range(raw: &str, raw_range: TextRange) -> Option<TextRange> {
    if !raw_range.is_non_empty() {
        return None;
    }
    let offset = frontmatter_body_char_offset(raw);
    if raw_range.start.offset() < offset {
        return None;
    }
    Some(TextRange::new(
        raw_range.start.offset() - offset,
        raw_range.end.offset() - offset,
    ))
}

/// Converts a stripped-body character range into raw Markdown coordinates for
/// source-editor highlighting and navigation.
pub fn body_range_to_raw_range(raw: &str, body_range: TextRange) -> TextRange {
    let offset = frontmatter_body_char_offset(raw);
    TextRange::new(
        body_range.start.offset().saturating_add(offset),
        body_range.end.offset().saturating_add(offset),
    )
}

struct FrontmatterSplit<'a> {
    frontmatter_block: &'a str,
    body_byte_offset: usize,
}

fn split_frontmatter_raw(raw: &str) -> Option<FrontmatterSplit<'_>> {
    let (open_line, after_open) = next_line(raw, 0)?;
    if open_line.trim_end_matches([' ', '\t']) != "---" {
        return None;
    }
    // Opening delimiter must end with a newline so a single-line `"---"` is not frontmatter.
    if after_open == open_line.len() {
        return None;
    }

    let mut cursor = after_open;
    while cursor < raw.len() {
        let (line, next_cursor) = next_line(raw, cursor)?;
        let trimmed = line.trim_end_matches([' ', '\t']);
        if trimmed == "---" || trimmed == "..." {
            let mut body_start = next_cursor;
            while body_start < raw.len() {
                let Some((blank_line, after_blank)) = next_line(raw, body_start) else {
                    break;
                };
                if blank_line.is_empty() {
                    body_start = after_blank;
                } else {
                    break;
                }
            }
            return Some(FrontmatterSplit {
                frontmatter_block: &raw[after_open..cursor],
                body_byte_offset: body_start,
            });
        }
        cursor = next_cursor;
    }

    None
}

fn next_line(text: &str, start: usize) -> Option<(&str, usize)> {
    if start >= text.len() {
        return None;
    }
    let rest = &text[start..];
    if let Some(newline_pos) = rest.find('\n') {
        let mut line = &rest[..newline_pos];
        if let Some(stripped) = line.strip_suffix('\r') {
            line = stripped;
        }
        Some((line, start + newline_pos + 1))
    } else {
        let line = rest.strip_suffix('\r').unwrap_or(rest);
        Some((line, text.len()))
    }
}

fn iter_frontmatter_entries(block: &str) -> impl Iterator<Item = (&str, &str)> {
    block.lines().filter_map(|raw_line| {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line).trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let (raw_key, raw_val) = line.split_once(':')?;
        let key = raw_key.trim();
        if key.is_empty() {
            return None;
        }
        let value = parse_yaml_scalar(raw_val)?;
        Some((key, value))
    })
}

fn parse_yaml_scalar(raw: &str) -> Option<&str> {
    let trimmed = raw.trim_matches([' ', '\t']);
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix('"') {
        let end = find_closing_quote(rest, '"')?;
        let inner = &rest[..end];
        return (!inner.trim().is_empty()).then_some(inner);
    }

    if let Some(rest) = trimmed.strip_prefix('\'') {
        let end = find_closing_quote(rest, '\'')?;
        let inner = &rest[..end];
        return (!inner.trim().is_empty()).then_some(inner);
    }

    let without_comment = strip_inline_comment(trimmed).trim_end_matches([' ', '\t']);
    if without_comment.is_empty() || matches!(without_comment, "~" | "null" | "Null" | "NULL") {
        return None;
    }
    Some(without_comment)
}

fn find_closing_quote(rest: &str, quote: char) -> Option<usize> {
    let mut escaped = false;
    for (idx, ch) in rest.char_indices() {
        if quote == '"' && ch == '\\' && !escaped {
            escaped = true;
            continue;
        }
        if ch == quote && !escaped {
            let trailing = rest[idx + ch.len_utf8()..].trim_matches([' ', '\t']);
            if trailing.is_empty() || trailing.starts_with('#') {
                return Some(idx);
            }
            return None;
        }
        escaped = false;
    }
    None
}

fn strip_inline_comment(value: &str) -> &str {
    let bytes = value.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i] == b'#' && matches!(bytes[i - 1], b' ' | b'\t') {
            return &value[..i];
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_fields_and_strips_body() {
        let raw = "---\ntype: bug\nstatus: testing\ntitle: \"Export timeout\"\nref: PROJ-42\n---\n\n# Export timeout\n\nBody text.\n";
        let parsed = parse_markdown_frontmatter(raw);
        assert_eq!(
            parsed.frontmatter,
            Some(DocumentFrontmatter {
                issue_type: Some("bug".into()),
                status: Some("testing".into()),
                title: Some("Export timeout".into()),
                external_ref: Some("PROJ-42".into()),
            })
        );
        assert_eq!(parsed.body, "# Export timeout\n\nBody text.\n");
        assert_eq!(&raw[parsed.body_byte_offset..], parsed.body);
        assert_eq!(
            raw.chars()
                .skip(parsed.body_char_offset as usize)
                .collect::<String>(),
            parsed.body
        );
    }

    #[test]
    fn leaves_markdown_without_frontmatter_untouched() {
        let raw = "# Login\n\n---\n\nBody\n";
        let parsed = parse_markdown_frontmatter(raw);
        assert_eq!(parsed.frontmatter, None);
        assert_eq!(parsed.body, raw);
        assert_eq!(parsed.body_byte_offset, 0);
        assert_eq!(parsed.body_char_offset, 0);
    }

    #[test]
    fn unclosed_frontmatter_is_treated_as_plain_markdown() {
        let raw = "---\nstatus: testing\n# Not closed\n";
        let parsed = parse_markdown_frontmatter(raw);
        assert_eq!(parsed.frontmatter, None);
        assert_eq!(parsed.body, raw);
    }

    #[test]
    fn supports_crlf_comments_single_quotes_and_unknown_keys() {
        let raw = "---\r\n# comment\r\ntype: feat # feature\r\nstatus: 'open'\r\nauthor: alice\r\nref: null\r\n---\r\n# Title\r\nBody\r\n";
        let parsed = parse_markdown_frontmatter(raw);
        assert_eq!(
            parsed.frontmatter,
            Some(DocumentFrontmatter {
                issue_type: Some("feat".into()),
                status: Some("open".into()),
                title: None,
                external_ref: None,
            })
        );
        assert_eq!(parsed.body, "# Title\r\nBody\r\n");
    }

    #[test]
    fn maps_between_raw_and_body_ranges() {
        let raw = "---\nstatus: testing\n---\n# 标题\n\n正文锚点\n";
        let offset = frontmatter_body_char_offset(raw);
        assert!(offset > 0);
        assert_eq!(raw_range_to_body_range(raw, TextRange::new(0, 5)), None);
        let body_range = TextRange::new(6, 10);
        let raw_range = body_range_to_raw_range(raw, body_range);
        assert_eq!(raw_range_to_body_range(raw, raw_range), Some(body_range));
    }

    #[test]
    fn issue_kind_and_done_status_helpers_handle_case_and_defaults() {
        assert_eq!(document_issue_kind("# Plain doc\n"), IssueKind::Feat);
        assert!(!document_is_done("# Plain doc\n"));
        assert_eq!(IssueKind::Feat.badge(), "[FEAT]");
        assert_eq!(IssueKind::Bug.badge(), "[BUG]");

        let bug_testing = "---\ntype: BUG\nstatus: testing\n---\n# Crash\n";
        assert_eq!(document_issue_kind(bug_testing), IssueKind::Bug);
        assert!(!document_is_done(bug_testing));

        let done_feat = "---\ntype: feat\nstatus: DONE\n---\n# Shipped\n";
        assert_eq!(document_issue_kind(done_feat), IssueKind::Feat);
        assert!(document_is_done(done_feat));
    }
}
