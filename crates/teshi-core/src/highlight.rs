//! Gherkin syntax highlighting without UI framework dependencies.

use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;

use crate::gherkin_lang::{GherkinLanguage, StepKeywordType, StructuralType};

/// Display width reserved for the step keyword gutter (after leading indent).
pub const STEP_KEYWORD_COL_WIDTH: usize = 6;

/// Semantic highlight category for one text span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HighlightKind {
    Default,
    Comment,
    Header,
    Tag,
    Given,
    When,
    Then,
    AndBut,
    String,
    Meta,
    DocString,
}

/// One styled segment of a source line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightSpan {
    pub text: String,
    pub kind: HighlightKind,
}

/// Count leading whitespace characters (not bytes) before the first non-whitespace.
pub fn leading_whitespace_chars(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

/// Extra spaces to insert before the keyword so the gutter is `STEP_KEYWORD_COL_WIDTH` wide.
pub fn step_keyword_gutter_pad(keyword: &str) -> usize {
    STEP_KEYWORD_COL_WIDTH.saturating_sub(keyword.width())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepMajor {
    Given,
    When,
    Then,
}

impl From<StepKeywordType> for StepMajor {
    fn from(t: StepKeywordType) -> Self {
        match t {
            StepKeywordType::Given => StepMajor::Given,
            StepKeywordType::When => StepMajor::When,
            StepKeywordType::Then => StepMajor::Then,
            StepKeywordType::And | StepKeywordType::But => StepMajor::Given,
        }
    }
}

impl StepMajor {
    fn highlight_kind(self) -> HighlightKind {
        match self {
            StepMajor::Given => HighlightKind::Given,
            StepMajor::When => HighlightKind::When,
            StepMajor::Then => HighlightKind::Then,
        }
    }
}

/// Tracks doc-string and And/But inheritance while highlighting consecutive lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StepHighlightState {
    pub in_doc_string: bool,
    last_major: Option<StepMajor>,
}

impl StepHighlightState {
    /// Creates highlight state with an initial doc-string flag.
    pub fn with_doc_string(in_doc_string: bool) -> Self {
        Self {
            in_doc_string,
            last_major: None,
        }
    }
}

/// Applies Gherkin-oriented highlighting for one buffer line, returning styled spans.
pub fn highlight_line_spans(
    line: &str,
    state: &mut StepHighlightState,
    lang: &GherkinLanguage,
) -> Vec<HighlightSpan> {
    let trimmed = line.trim_start();
    let is_doc_marker = trimmed.starts_with("\"\"\"");

    if is_doc_marker {
        state.in_doc_string = !state.in_doc_string;
        return vec![span(line, HighlightKind::DocString)];
    }
    if state.in_doc_string {
        return vec![span(line, HighlightKind::DocString)];
    }
    if trimmed.starts_with('#') {
        return vec![span(line, HighlightKind::Comment)];
    }
    if trimmed.starts_with('|') {
        return vec![span(line, HighlightKind::Meta)];
    }
    if trimmed.starts_with('@') {
        return line
            .split_whitespace()
            .map(|part| {
                let kind = if part.starts_with('@') {
                    HighlightKind::Tag
                } else {
                    HighlightKind::Default
                };
                span(part, kind)
            })
            .collect();
    }

    if let Some((_kw, st)) = lang.match_structural_prefix(trimmed)
        && matches!(
            st,
            StructuralType::Feature
                | StructuralType::Scenario
                | StructuralType::ScenarioOutline
                | StructuralType::Background
        )
    {
        state.last_major = None;
    }

    if let Some((matched, _st)) = lang.match_structural_prefix(trimmed) {
        let leading = leading_whitespace_chars(line);
        let mut spans = Vec::new();
        if leading > 0 {
            spans.push(span(
                &line.chars().take(leading).collect::<String>(),
                HighlightKind::Default,
            ));
        }
        let rest: String = trimmed.chars().skip(matched.chars().count()).collect();
        spans.push(span(matched, HighlightKind::Header));
        spans.push(span(&rest, HighlightKind::Default));
        return spans;
    }

    if let Some((matched, kw_type)) = lang.match_step_prefix(trimmed) {
        let leading = leading_whitespace_chars(line);
        let mut spans = Vec::new();
        if leading > 0 {
            spans.push(span(
                &line.chars().take(leading).collect::<String>(),
                HighlightKind::Default,
            ));
        }
        let rest: String = trimmed.chars().skip(matched.chars().count()).collect();

        let kind = match kw_type {
            StepKeywordType::Given => {
                state.last_major = Some(StepMajor::Given);
                HighlightKind::Given
            }
            StepKeywordType::When => {
                state.last_major = Some(StepMajor::When);
                HighlightKind::When
            }
            StepKeywordType::Then => {
                state.last_major = Some(StepMajor::Then);
                HighlightKind::Then
            }
            StepKeywordType::And | StepKeywordType::But => state
                .last_major
                .map(StepMajor::highlight_kind)
                .unwrap_or(HighlightKind::AndBut),
        };

        let pad = step_keyword_gutter_pad(matched);
        let kw_text = if pad == 0 {
            matched.to_string()
        } else {
            format!("{}{}", " ".repeat(pad), matched)
        };
        spans.push(span(&kw_text, kind));
        spans.push(span(&rest, HighlightKind::Default));
        return spans;
    }

    let mut spans = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            let mut buf = String::from("\"");
            for c in chars.by_ref() {
                buf.push(c);
                if c == '"' {
                    break;
                }
            }
            spans.push(span(&buf, HighlightKind::String));
        } else {
            spans.push(span(&ch.to_string(), HighlightKind::Default));
        }
    }
    spans
}

fn span(text: &str, kind: HighlightKind) -> HighlightSpan {
    HighlightSpan {
        text: text.to_string(),
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gherkin_lang::GherkinLanguages;

    fn en() -> &'static GherkinLanguage {
        GherkinLanguages::global().get("en")
    }

    #[test]
    fn test_highlight_header_span() {
        let mut state = StepHighlightState::default();
        let spans = highlight_line_spans("Feature: Login", &mut state, en());
        assert!(
            spans
                .iter()
                .any(|s| s.text == "Feature:" && s.kind == HighlightKind::Header)
        );
    }

    #[test]
    fn test_and_inherits_previous_major_kind() {
        let mut state = StepHighlightState::default();
        let _ = highlight_line_spans("When I log in", &mut state, en());
        let and_spans = highlight_line_spans("And I see home", &mut state, en());
        assert!(and_spans.iter().any(|s| s.kind == HighlightKind::When));
    }
}
