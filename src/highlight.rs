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

/// Applies Gherkin-oriented foreground highlighting for one buffer line.
pub fn highlight_line(
    line: &str,
    in_doc_string: bool,
    keywords: &KeywordSet,
) -> (Line<'static>, bool) {
    let default = Style::default();
    let comment = Style::default().fg(Color::DarkGray);
    let header = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let step = Style::default().fg(Color::Magenta);
    let tag = Style::default().fg(Color::Yellow);
    let string = Style::default().fg(Color::Green);
    let meta = Style::default().fg(Color::Blue);

    let trimmed = line.trim_start();
    let is_doc_marker = trimmed.starts_with("\"\"\"");
    let mut next_doc_state = in_doc_string;

    if is_doc_marker {
        next_doc_state = !in_doc_string;
        return (
            Line::from(vec![Span::styled(line.to_string(), meta)]),
            next_doc_state,
        );
    }
    if in_doc_string {
        return (
            Line::from(vec![Span::styled(line.to_string(), meta)]),
            next_doc_state,
        );
    }
    if trimmed.starts_with('#') {
        return (
            Line::from(vec![Span::styled(line.to_string(), comment)]),
            next_doc_state,
        );
    }
    if trimmed.starts_with('|') {
        return (
            Line::from(vec![Span::styled(line.to_string(), meta)]),
            next_doc_state,
        );
    }
    if trimmed.starts_with('@') {
        let spans = line
            .split_whitespace()
            .map(|part| {
                let style = if part.starts_with('@') { tag } else { default };
                Span::styled(part.to_string(), style)
            })
            .collect::<Vec<_>>();
        return (Line::from(spans), next_doc_state);
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
            return (Line::from(spans), next_doc_state);
        }
    }
    for kw in keywords.steps {
        if let Some(stripped) = trimmed.strip_prefix(kw) {
            let leading_ws = line.len().saturating_sub(trimmed.len());
            let mut spans = Vec::new();
            if leading_ws > 0 {
                spans.push(Span::raw(" ".repeat(leading_ws)));
            }
            spans.push(Span::styled((*kw).to_string(), step));
            spans.push(Span::raw(stripped.to_string()));
            return (Line::from(spans), next_doc_state);
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
    (Line::from(spans), next_doc_state)
}

#[cfg(test)]
mod tests {
    use super::{KeywordSet, highlight_line};

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
}
