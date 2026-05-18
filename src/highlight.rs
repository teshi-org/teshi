use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::gherkin_lang::{GherkinLanguage, StepKeywordType, StructuralType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepMajor {
    Given,
    When,
    Then,
}

impl StepMajor {
    fn color(self) -> Color {
        match self {
            StepMajor::Given => Color::Blue,
            StepMajor::When => Color::Yellow,
            StepMajor::Then => Color::Green,
        }
    }
}

impl From<StepKeywordType> for StepMajor {
    fn from(t: StepKeywordType) -> Self {
        match t {
            StepKeywordType::Given => StepMajor::Given,
            StepKeywordType::When => StepMajor::When,
            StepKeywordType::Then => StepMajor::Then,
            StepKeywordType::And | StepKeywordType::But => {
                // Fallback; callers that pass And/But should handle separately
                StepMajor::Given
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StepHighlightState {
    pub in_doc_string: bool,
    pub last_major: Option<StepMajor>,
}

/// Applies Gherkin-oriented foreground highlighting for one buffer line.
#[cfg(test)]
pub fn highlight_line(
    line: &str,
    in_doc_string: bool,
    lang: &GherkinLanguage,
) -> (Line<'static>, bool) {
    let mut state = StepHighlightState {
        in_doc_string,
        last_major: None,
    };
    let line = highlight_line_with_state(line, &mut state, lang);
    (line, state.in_doc_string)
}

pub fn highlight_line_with_state(
    line: &str,
    state: &mut StepHighlightState,
    lang: &GherkinLanguage,
) -> Line<'static> {
    let default = Style::default();
    let comment = Style::default().fg(Color::DarkGray);
    let header = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let step_default = Style::default().fg(Color::Magenta);
    let tag = Style::default().fg(Color::Yellow);
    let string = Style::default().fg(Color::Green);
    let meta = Style::default().fg(Color::Blue);

    let trimmed = line.trim_start();
    let is_doc_marker = trimmed.starts_with("\"\"\"");

    if is_doc_marker {
        state.in_doc_string = !state.in_doc_string;
        return Line::from(vec![Span::styled(line.to_string(), meta)]);
    }
    if state.in_doc_string {
        return Line::from(vec![Span::styled(line.to_string(), meta)]);
    }
    if trimmed.starts_with('#') {
        return Line::from(vec![Span::styled(line.to_string(), comment)]);
    }
    if trimmed.starts_with('|') {
        return Line::from(vec![Span::styled(line.to_string(), meta)]);
    }
    if trimmed.starts_with('@') {
        let spans = line
            .split_whitespace()
            .map(|part| {
                let style = if part.starts_with('@') { tag } else { default };
                Span::styled(part.to_string(), style)
            })
            .collect::<Vec<_>>();
        return Line::from(spans);
    }

    // Reset major step type on scenario/feature boundaries
    if let Some((_kw, st)) = lang.match_structural_prefix(trimmed) {
        if matches!(
            st,
            StructuralType::Feature
                | StructuralType::Scenario
                | StructuralType::ScenarioOutline
                | StructuralType::Background
        ) {
            state.last_major = None;
        }
    }

    // Structural header highlighting
    if let Some((matched, _st)) = lang.match_structural_prefix(trimmed) {
        let leading_ws = line.len().saturating_sub(trimmed.len());
        let mut spans = Vec::new();
        if leading_ws > 0 {
            spans.push(Span::raw(" ".repeat(leading_ws)));
        }
        let kw_text = matched.to_string();
        let rest = &trimmed[kw_text.len()..];
        spans.push(Span::styled(kw_text, header));
        spans.push(Span::raw(rest.to_string()));
        return Line::from(spans);
    }

    // Step keyword highlighting
    if let Some((matched, kw_type)) = lang.match_step_prefix(trimmed) {
        let leading_ws = line.len().saturating_sub(trimmed.len());
        let mut spans = Vec::new();
        if leading_ws > 0 {
            spans.push(Span::raw(" ".repeat(leading_ws)));
        }
        let kw_text = matched.to_string();
        let rest = &trimmed[kw_text.len()..];

        let step_style = match kw_type {
            StepKeywordType::Given => {
                state.last_major = Some(StepMajor::Given);
                Style::default().fg(StepMajor::Given.color())
            }
            StepKeywordType::When => {
                state.last_major = Some(StepMajor::When);
                Style::default().fg(StepMajor::When.color())
            }
            StepKeywordType::Then => {
                state.last_major = Some(StepMajor::Then);
                Style::default().fg(StepMajor::Then.color())
            }
            StepKeywordType::And | StepKeywordType::But => {
                if let Some(major) = state.last_major {
                    Style::default().fg(major.color())
                } else {
                    Style::default().fg(Color::Gray)
                }
            }
        };

        spans.push(Span::styled(kw_text, step_style));
        spans.push(Span::raw(rest.to_string()));
        return Line::from(spans);
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
            spans.push(Span::styled(buf, string));
        } else {
            spans.push(Span::styled(ch.to_string(), default));
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gherkin_lang::GherkinLanguages;
    use ratatui::style::Color;

    fn en() -> &'static GherkinLanguage {
        GherkinLanguages::global().get("en")
    }

    fn keyword_fg(line: &Line<'_>, kw: &str) -> Option<Color> {
        line.spans
            .iter()
            .find(|s| s.content.as_ref() == kw)
            .and_then(|s| s.style.fg)
    }

    #[test]
    fn test_highlight_header() {
        let (line, _) = highlight_line("Feature: Login", false, en());
        assert_eq!(line.spans[0].content.as_ref(), "Feature:");
    }

    #[test]
    fn test_highlight_comment() {
        let (line, _) = highlight_line("# comment", false, en());
        assert_eq!(line.spans[0].content.as_ref(), "# comment");
    }

    #[test]
    fn test_doc_string_toggle() {
        let (_, in_doc) = highlight_line("\"\"\"", false, en());
        assert!(in_doc);
        let (_, in_doc_2) = highlight_line("\"\"\"", in_doc, en());
        assert!(!in_doc_2);
    }

    #[test]
    fn test_and_inherits_previous_major_color() {
        let mut state = StepHighlightState::default();
        let line1 = highlight_line_with_state("When I log in", &mut state, en());
        assert_eq!(keyword_fg(&line1, "When"), Some(Color::Yellow));
        let line2 = highlight_line_with_state("And I see home", &mut state, en());
        assert_eq!(keyword_fg(&line2, "And"), Some(Color::Yellow));
        let line3 = highlight_line_with_state("Then I log out", &mut state, en());
        assert_eq!(keyword_fg(&line3, "Then"), Some(Color::Green));
        let line4 = highlight_line_with_state("And I see login", &mut state, en());
        assert_eq!(keyword_fg(&line4, "And"), Some(Color::Green));
    }

    #[test]
    fn test_and_resets_on_new_scenario() {
        let mut state = StepHighlightState::default();
        let _ = highlight_line_with_state("Given A", &mut state, en());
        let _ = highlight_line_with_state("Scenario: Next", &mut state, en());
        let line = highlight_line_with_state("And B", &mut state, en());
        assert_eq!(keyword_fg(&line, "And"), Some(Color::Gray));
    }
}
