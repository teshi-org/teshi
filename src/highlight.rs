use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use teshi_gherkin::gherkin_lang::GherkinLanguage;
use teshi_gherkin::highlight::{HighlightKind, highlight_line_spans};
pub use teshi_gherkin::highlight::{
    STEP_KEYWORD_COL_WIDTH, StepHighlightState, leading_whitespace_chars, step_keyword_gutter_pad,
};

/// Styled gutter span used by Explore Steps and the BDD Editor: pad spaces + keyword in one span.
pub(crate) fn step_keyword_gutter_styled_span(
    keyword: &str,
    style: Style,
) -> (Span<'static>, usize) {
    let pad = step_keyword_gutter_pad(keyword);
    let text = if pad == 0 {
        keyword.to_string()
    } else {
        format!("{}{}", " ".repeat(pad), keyword)
    };
    (Span::styled(text, style), pad)
}

/// Applies Gherkin-oriented foreground highlighting for one buffer line.
#[cfg(test)]
pub fn highlight_line(
    line: &str,
    in_doc_string: bool,
    lang: &GherkinLanguage,
) -> (Line<'static>, bool) {
    let mut state = StepHighlightState::with_doc_string(in_doc_string);
    let rendered = highlight_line_with_state(line, &mut state, lang);
    (rendered, state.in_doc_string)
}

pub fn highlight_line_with_state(
    line: &str,
    state: &mut StepHighlightState,
    lang: &GherkinLanguage,
) -> Line<'static> {
    let spans: Vec<Span<'static>> = highlight_line_spans(line, state, lang)
        .into_iter()
        .map(|span| Span::styled(span.text, kind_to_style(span.kind)))
        .collect();
    Line::from(spans)
}

fn kind_to_style(kind: HighlightKind) -> Style {
    match kind {
        HighlightKind::Default => Style::default(),
        HighlightKind::Comment => Style::default().fg(Color::DarkGray),
        HighlightKind::Header => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        HighlightKind::Tag => Style::default().fg(Color::Yellow),
        HighlightKind::Given => Style::default().fg(Color::Blue),
        HighlightKind::When => Style::default().fg(Color::Yellow),
        HighlightKind::Then => Style::default().fg(Color::Green),
        HighlightKind::AndBut => Style::default().fg(Color::Gray),
        HighlightKind::String => Style::default().fg(Color::Green),
        HighlightKind::Meta | HighlightKind::DocString => Style::default().fg(Color::Blue),
    }
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
            .find(|s| s.style.fg.is_some() && s.content.as_ref().ends_with(kw))
            .and_then(|s| s.style.fg)
    }

    #[test]
    fn test_highlight_header() {
        let (line, _) = highlight_line("Feature: Login", false, en());
        assert_eq!(line.spans[0].content.as_ref(), "Feature:");
    }

    #[test]
    fn test_highlight_background_header_en_and_zh() {
        let en_line = highlight_line("  Background:", false, en()).0;
        assert!(
            en_line
                .spans
                .iter()
                .any(|s| s.content.as_ref().contains("Background:")),
            "expected Background: span, got {:?}",
            en_line.spans
        );
        let zh = GherkinLanguages::global().get("zh-CN");
        let zh_line = highlight_line("  背景:", false, zh).0;
        assert!(
            zh_line
                .spans
                .iter()
                .any(|s| s.content.as_ref().contains("背景:")),
            "expected 背景: span, got {:?}",
            zh_line.spans
        );
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

    fn gutter_keyword_end_col(line: &Line<'_>) -> usize {
        use unicode_width::UnicodeWidthStr;
        let mut col = 0usize;
        for span in &line.spans {
            if span.style.fg.is_some() {
                return col + span.width();
            }
            col += span.width();
        }
        0
    }

    #[test]
    fn test_editor_zh_gutter_single_span_and_并且_right_edge() {
        let zh = GherkinLanguages::global().get("zh-CN");
        let cases = [
            ("          当 用户", "    当"),
            ("          那么 结果", "  那么"),
            ("          并且 其他", "  并且"),
            ("          而且 其他", "  而且"),
            ("          假如 存在", "  假如"),
        ];
        let leading = leading_whitespace_chars(cases[0].0);
        let mut prev_end = None;
        for (input, expected_gutter) in cases {
            let mut state = StepHighlightState::default();
            let line = highlight_line_with_state(input, &mut state, zh);
            let end = gutter_keyword_end_col(&line);
            assert_eq!(
                end,
                leading + STEP_KEYWORD_COL_WIDTH,
                "gutter right edge for '{input}'"
            );
            let styled: Vec<_> = line.spans.iter().filter(|s| s.style.fg.is_some()).collect();
            assert_eq!(
                styled.len(),
                1,
                "expected one styled gutter span for '{input}'"
            );
            assert_eq!(
                styled[0].content.as_ref(),
                expected_gutter,
                "gutter span content for '{input}'"
            );
            if let Some(prev) = prev_end {
                assert_eq!(end, prev);
            }
            prev_end = Some(end);
        }
    }
}
