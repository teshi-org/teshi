use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub struct KeywordSet {
    pub headers: &'static [&'static str],
    pub steps: &'static [&'static str],
}

impl Default for KeywordSet {
    fn default() -> Self {
        Self {
            headers: &[
                "Feature:",
                "Background:",
                "Scenario:",
                "Scenario Outline:",
                "Examples:",
            ],
            steps: &["Given", "When", "Then", "And", "But"],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepMajor {
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
    keywords: &KeywordSet,
) -> (Line<'static>, bool) {
    let mut state = StepHighlightState {
        in_doc_string,
        last_major: None,
    };
    let line = highlight_line_with_state(line, &mut state, keywords);
    (line, state.in_doc_string)
}

pub fn highlight_line_with_state(
    line: &str,
    state: &mut StepHighlightState,
    keywords: &KeywordSet,
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

    let reset_major = trimmed.starts_with("Feature:")
        || trimmed.starts_with("Background:")
        || trimmed.starts_with("Scenario Outline:")
        || trimmed.starts_with("Scenario:");
    if reset_major {
        state.last_major = None;
    }

    for kw in keywords.headers {
        if let Some(stripped) = trimmed.strip_prefix(kw) {
            let leading_ws = line.len().saturating_sub(trimmed.len());
            let mut spans = Vec::new();
            if leading_ws > 0 {
                spans.push(Span::raw(" ".repeat(leading_ws)));
            }
            spans.push(Span::styled((*kw).to_string(), header));
            spans.push(Span::raw(stripped.to_string()));
            return Line::from(spans);
        }
    }
    for kw in keywords.steps {
        if let Some(stripped) = trimmed.strip_prefix(kw) {
            let leading_ws = line.len().saturating_sub(trimmed.len());
            let mut spans = Vec::new();
            if leading_ws > 0 {
                spans.push(Span::raw(" ".repeat(leading_ws)));
            }
            let step_style = match *kw {
                "Given" => {
                    state.last_major = Some(StepMajor::Given);
                    Style::default().fg(StepMajor::Given.color())
                }
                "When" => {
                    state.last_major = Some(StepMajor::When);
                    Style::default().fg(StepMajor::When.color())
                }
                "Then" => {
                    state.last_major = Some(StepMajor::Then);
                    Style::default().fg(StepMajor::Then.color())
                }
                "And" | "But" => {
                    if let Some(major) = state.last_major {
                        Style::default().fg(major.color())
                    } else {
                        Style::default().fg(Color::Gray)
                    }
                }
                _ => step_default,
            };
            spans.push(Span::styled((*kw).to_string(), step_style));
            spans.push(Span::raw(stripped.to_string()));
            return Line::from(spans);
        }
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
    use super::{KeywordSet, StepHighlightState, highlight_line, highlight_line_with_state};
    use ratatui::style::Color;
    use ratatui::text::Line;

    #[test]
    fn test_highlight_header() {
        let (line, _) = highlight_line("Feature: Login", false, &KeywordSet::default());
        assert_eq!(line.spans[0].content.as_ref(), "Feature:");
    }

    #[test]
    fn test_highlight_comment() {
        let (line, _) = highlight_line("# comment", false, &KeywordSet::default());
        assert_eq!(line.spans[0].content.as_ref(), "# comment");
    }

    #[test]
    fn test_doc_string_toggle() {
        let (_, in_doc) = highlight_line("\"\"\"", false, &KeywordSet::default());
        assert!(in_doc);
        let (_, in_doc_2) = highlight_line("\"\"\"", in_doc, &KeywordSet::default());
        assert!(!in_doc_2);
    }

    fn keyword_fg(line: &Line<'_>, kw: &str) -> Option<Color> {
        line.spans
            .iter()
            .find(|s| s.content.as_ref() == kw)
            .and_then(|s| s.style.fg)
    }

    #[test]
    fn test_and_inherits_previous_major_color() {
        let mut state = StepHighlightState::default();
        let line1 = highlight_line_with_state("When I log in", &mut state, &KeywordSet::default());
        assert_eq!(keyword_fg(&line1, "When"), Some(Color::Yellow));
        let line2 = highlight_line_with_state("And I see home", &mut state, &KeywordSet::default());
        assert_eq!(keyword_fg(&line2, "And"), Some(Color::Yellow));
        let line3 = highlight_line_with_state("Then I log out", &mut state, &KeywordSet::default());
        assert_eq!(keyword_fg(&line3, "Then"), Some(Color::Green));
        let line4 =
            highlight_line_with_state("And I see login", &mut state, &KeywordSet::default());
        assert_eq!(keyword_fg(&line4, "And"), Some(Color::Green));
    }

    #[test]
    fn test_and_resets_on_new_scenario() {
        let mut state = StepHighlightState::default();
        let _ = highlight_line_with_state("Given A", &mut state, &KeywordSet::default());
        let _ = highlight_line_with_state("Scenario: Next", &mut state, &KeywordSet::default());
        let line = highlight_line_with_state("And B", &mut state, &KeywordSet::default());
        assert_eq!(keyword_fg(&line, "And"), Some(Color::Gray));
    }
}
