use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use std::collections::HashMap;

/// Per-level heading styles: larger levels get less emphasis.
fn heading_styles() -> HashMap<usize, Style> {
    let mut m = HashMap::new();
    m.insert(
        1,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );
    m.insert(
        2,
        Style::default()
            .fg(Color::Rgb(220, 220, 255))
            .add_modifier(Modifier::BOLD),
    );
    m.insert(
        3,
        Style::default()
            .fg(Color::Rgb(200, 200, 230))
            .add_modifier(Modifier::BOLD),
    );
    m.insert(
        4,
        Style::default()
            .fg(Color::Rgb(180, 180, 210))
            .add_modifier(Modifier::BOLD),
    );
    m.insert(5, Style::default().fg(Color::Rgb(160, 160, 190)));
    m.insert(6, Style::default().fg(Color::Rgb(140, 140, 170)));
    m
}

/// Renders a Markdown text string into styled `ratatui` lines.
///
/// Supports block-level elements (headings, lists, blockquotes, fenced code blocks,
/// tables, and paragraphs) and inline formatting (bold, italic, inline code, links,
/// and strikethrough).
///
/// # Examples
///
/// ```
/// use teshi::markdown::render_markdown;
/// let lines = render_markdown("Hello **world**");
/// assert_eq!(lines.len(), 1);
/// ```
pub fn render_markdown(text: &str) -> Vec<Line<'static>> {
    // Strip <think>...</think> blocks (reasoning model output)
    let text = strip_think_tags(text);
    let mut lines = Vec::new();
    let input = &text;
    let mut pos = 0usize;
    let mut needs_sep = false;

    while pos < input.len() {
        // Skip leading blank lines.
        let skipped = skip_empty_lines(&input[pos..]);
        pos += skipped;
        if pos >= input.len() {
            break;
        }
        let remaining = &input[pos..];

        // Insert a blank line between consecutive blocks for visual breathing room.
        if needs_sep {
            let last = lines.last();
            let already_blank = last.is_some_and(|l: &Line| {
                l.spans.iter().all(|s| s.content.as_ref().is_empty()) && l.style == Style::default()
            });
            if !already_blank {
                lines.push(Line::raw(""));
            }
        }

        // Detect block type by the first non-empty line.
        if let Some(block) = try_fenced_code_block(remaining) {
            pos += block.consumed;
            lines.extend(block.lines);
        } else if let Some(block) = try_heading(remaining) {
            pos += block.consumed;
            lines.push(block.line);
        } else if let Some(block) = try_horizontal_rule(remaining) {
            pos += block.consumed;
            lines.push(block.line);
        } else if let Some(block) = try_blockquote(remaining) {
            pos += block.consumed;
            lines.extend(block.lines);
        } else if let Some(block) = try_list(remaining) {
            pos += block.consumed;
            lines.extend(block.lines);
        } else if let Some(block) = try_table(remaining) {
            pos += block.consumed;
            lines.extend(block.lines);
        } else {
            let block = take_paragraph(remaining);
            pos += block.consumed;
            for line in block.lines {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                lines.push(parse_inline(&text));
            }
        }

        needs_sep = true;
    }

    lines
}

/// Strips `<think>...</think>` blocks (with content inside) from reasoning models.
fn strip_think_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_think = false;
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < text.len() {
        if bytes[i..].starts_with(b"<think>") {
            in_think = true;
            i += 7;
        } else if in_think && bytes[i..].starts_with(b"</think>") {
            in_think = false;
            i += 8;
        } else if !in_think {
            result.push(text[i..].chars().next().unwrap());
            i += text[i..].chars().next().unwrap().len_utf8();
        } else {
            i += 1;
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Block parsers
// ---------------------------------------------------------------------------

struct BlockResult {
    consumed: usize,
    lines: Vec<Line<'static>>,
}

struct SingleLineResult {
    consumed: usize,
    line: Line<'static>,
}

fn skip_empty_lines(text: &str) -> usize {
    let mut count = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            count += line.len() + 1; // +1 for '\n'
        } else {
            break;
        }
    }
    // Adjust if the text ends with newlines (no trailing '\n' on last line).
    count = count.min(text.len());
    count
}

fn try_fenced_code_block(text: &str) -> Option<BlockResult> {
    let first_line = text.lines().next()?;
    let trimmed = first_line.trim_start();
    if !trimmed.starts_with("```") && !trimmed.starts_with("~~~") {
        return None;
    }
    let fence_char = trimmed.chars().next()?;
    let fence_len = trimmed.chars().take_while(|&c| c == fence_char).count();
    if fence_len < 3 {
        return None;
    }

    // Extract language from the info string after the fence opener
    let lang = &trimmed[fence_len..].trim();
    let has_lang = !lang.is_empty();

    // Clamp to text length: if there is no trailing '\n', first_line consumes the whole text.
    let mut consumed = (first_line.len() + 1).min(text.len());
    let mut code_lines: Vec<String> = Vec::new();
    let rest = &text[consumed..];

    for line in rest.lines() {
        let t = line.trim_start();
        if t.chars().take_while(|&c| c == fence_char).count() >= fence_len {
            consumed += line.len() + 1;
            break;
        }
        code_lines.push(line.to_string());
        consumed += line.len() + 1;
    }

    // Clamp consumed to text length.
    consumed = consumed.min(text.len());

    let bg = Style::default().bg(Color::Rgb(30, 30, 30)).fg(Color::Gray);
    let mut out: Vec<Line<'static>> = Vec::with_capacity(code_lines.len().saturating_add(2));

    // Language label line (if present)
    if has_lang {
        let label_style = Style::default()
            .bg(Color::Rgb(30, 30, 30))
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD);
        let mut label_line = Line::styled(format!(" {} ", lang), label_style);
        label_line.style = bg;
        out.push(label_line);
    }

    for code in code_lines {
        let mut line = Line::from(Span::styled(code, bg));
        line.style = bg;
        out.push(line);
    }

    Some(BlockResult {
        consumed,
        lines: out,
    })
}

fn try_heading(text: &str) -> Option<SingleLineResult> {
    let first_line = text.lines().next()?;
    let trimmed = first_line.trim_start();
    let level = trimmed.chars().take_while(|&c| c == '#').count();
    if level == 0 || level > 6 || trimmed.chars().nth(level) != Some(' ') {
        return None;
    }
    let content = &trimmed[level + 1..];
    let styles = heading_styles();
    let heading_style = styles.get(&level).copied().unwrap_or(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );
    let line = parse_inline_with_style(content, heading_style);
    Some(SingleLineResult {
        consumed: first_line.len() + 1,
        line,
    })
}

/// Detects and renders a horizontal rule: `---`, `***`, or `___` (3+ characters).
fn try_horizontal_rule(text: &str) -> Option<SingleLineResult> {
    let line = text.lines().next()?;
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return None;
    }
    let ch = trimmed.chars().next()?;
    if (ch == '-' || ch == '*' || ch == '_') && trimmed.chars().all(|c| c == ch) {
        // Repeat the horizontal rule character to fill available width (use a fixed count)
        let rule = "─".repeat(50); // arbitrary fill
        Some(SingleLineResult {
            consumed: line.len() + 1,
            line: Line::styled(rule, Style::default().fg(Color::DarkGray)),
        })
    } else {
        None
    }
}

fn try_blockquote(text: &str) -> Option<BlockResult> {
    let first_line = text.lines().next()?;
    if !first_line.trim_start().starts_with('>') {
        return None;
    }

    let mut consumed = 0usize;
    let mut raw_lines: Vec<String> = Vec::new();

    for line in text.lines() {
        let t = line.trim_start();
        if !t.starts_with('>') && !line.trim().is_empty() {
            break;
        }
        if let Some(after) = t.strip_prefix('>') {
            let after = after.strip_prefix(' ').unwrap_or(after);
            raw_lines.push(after.to_string());
        } else {
            // blank line inside blockquote
            raw_lines.push(String::new());
        }
        consumed += line.len() + 1;
    }

    consumed = consumed.min(text.len());

    let base_style = Style::default()
        .fg(Color::Gray)
        .add_modifier(Modifier::ITALIC);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(raw_lines.len());
    for raw in raw_lines {
        let mut line = parse_inline_with_style(&raw, base_style);
        // Prefix with a blockquote indicator styled dimly.
        let mut spans = vec![Span::styled("│ ", Style::default().fg(Color::DarkGray))];
        spans.extend(
            line.spans
                .into_iter()
                .map(|s| Span::styled(s.content.to_string(), s.style)),
        );
        line = Line::from(spans);
        lines.push(line);
    }

    Some(BlockResult { consumed, lines })
}

fn try_list(text: &str) -> Option<BlockResult> {
    let first_line = text.lines().next()?;
    let (is_ordered, indent, marker_len) = list_marker_info(first_line)?;

    let mut consumed = 0usize;
    let mut items: Vec<Vec<String>> = Vec::new();
    let mut current_item: Vec<String> = Vec::new();

    for line in text.lines() {
        let line_indent = leading_spaces(line);
        let t = line.trim_start();

        if line.trim().is_empty() {
            current_item.push(String::new());
            consumed += line.len() + 1;
            continue;
        }

        if line_indent < indent && !t.is_empty() {
            break;
        }

        if line_indent == indent
            && let Some((ordered, _i, mlen)) = list_marker_info(line)
            && ordered == is_ordered
        {
            if !current_item.is_empty() {
                items.push(current_item);
                current_item = Vec::new();
            }
            current_item.push(t[mlen..].to_string());
            consumed += line.len() + 1;
            continue;
        }

        // Continuation line: preserve relative indentation.
        let dedented = if line_indent >= indent + marker_len {
            &line[indent + marker_len..]
        } else {
            t
        };
        current_item.push(dedented.to_string());
        consumed += line.len() + 1;
    }

    if !current_item.is_empty() {
        items.push(current_item);
    }

    if items.is_empty() {
        return None;
    }

    consumed = consumed.min(text.len());

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (idx, item_lines) in items.iter().enumerate() {
        let first = &item_lines[0];
        // Check for task list: "- [x] ..." or "- [ ] ..."
        let (bullet, checked, first_text) = if is_ordered {
            (format!("{}. ", idx + 1), None, first.as_str())
        } else if let Some(rest) = first
            .strip_prefix("[x] ")
            .or_else(|| first.strip_prefix("[X] "))
        {
            ("☑ ".to_string(), Some(true), rest)
        } else if let Some(rest) = first.strip_prefix("[ ] ") {
            ("☐ ".to_string(), Some(false), rest)
        } else {
            ("• ".to_string(), None, first.as_str())
        };
        let bullet_style = match checked {
            Some(true) => Style::default().fg(Color::Green),
            Some(false) => Style::default().fg(Color::DarkGray),
            None => Style::default().fg(Color::Yellow),
        };
        let bullet_span = Span::styled(bullet, bullet_style);

        for (li, raw) in item_lines.iter().enumerate() {
            let text = if li == 0 { first_text } else { raw };
            let inline = parse_inline(text);
            let mut spans = Vec::new();
            if li == 0 {
                spans.push(bullet_span.clone());
            } else {
                spans.push(Span::styled("   ", Style::default()));
            }
            spans.extend(inline.spans);
            lines.push(Line::from(spans));
        }
    }

    Some(BlockResult { consumed, lines })
}

fn try_table(text: &str) -> Option<BlockResult> {
    let mut consumed = 0usize;
    let mut raw_lines: Vec<String> = Vec::new();

    for line in text.lines() {
        let t = line.trim();
        let pipe_count = t.chars().filter(|&c| c == '|').count();
        if raw_lines.is_empty() {
            // First line must have at least 2 pipes to form a table.
            if pipe_count >= 2 {
                raw_lines.push(t.to_string());
                consumed += line.len() + 1;
            } else {
                break;
            }
        } else if pipe_count >= 1 {
            raw_lines.push(t.to_string());
            consumed += line.len() + 1;
        } else if line.trim().is_empty() {
            consumed += line.len() + 1;
            continue;
        } else {
            break;
        }
    }

    if raw_lines.len() < 2 {
        return None;
    }

    // The second row must be a separator line (contains only `|`, `-`, `:` and spaces).
    let sep = &raw_lines[1];
    if !is_table_separator(sep) {
        return None;
    }

    consumed = consumed.min(text.len());

    // Parse all rows into cells.
    let rows: Vec<Vec<String>> = raw_lines
        .iter()
        .map(|line| split_table_cells(line))
        .collect();

    if rows.is_empty() || rows.iter().any(|r| r.len() != rows[0].len()) {
        return None;
    }

    let col_count = rows[0].len();
    let mut col_widths: Vec<usize> = vec![0; col_count];
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            let w = UnicodeWidthStr::width(cell.trim());
            col_widths[i] = col_widths[i].max(w);
        }
    }

    let header_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let cell_style = Style::default().fg(Color::Gray);
    let sep_style = Style::default().fg(Color::DarkGray);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(rows.len());

    for (ri, row) in rows.iter().enumerate() {
        if ri == 1 {
            // Separator row: render as a dashed line.
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (ci, width) in col_widths.iter().enumerate() {
                if ci > 0 {
                    spans.push(Span::styled("|", sep_style));
                }
                let dash_len = width.saturating_add(2); // +2 for padding spaces
                spans.push(Span::styled("-".repeat(dash_len), sep_style));
            }
            lines.push(Line::from(spans));
            continue;
        }

        let base_style = if ri == 0 { header_style } else { cell_style };
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (ci, cell) in row.iter().enumerate() {
            if ci > 0 {
                spans.push(Span::styled("|", sep_style));
            }
            let trimmed = cell.trim();
            let w = UnicodeWidthStr::width(trimmed);
            let pad = col_widths[ci].saturating_sub(w);
            let padded = format!(" {}{} ", trimmed, " ".repeat(pad));
            // Apply inline Markdown parsing to cell content with the row base style.
            let cell_line = parse_inline_with_style(&padded, base_style);
            spans.extend(cell_line.spans);
        }
        lines.push(Line::from(spans));
    }

    Some(BlockResult { consumed, lines })
}

/// Checks whether a line is a table separator (only `|`, `-`, `:`, and spaces).
fn is_table_separator(line: &str) -> bool {
    line.trim()
        .chars()
        .all(|c| c == '|' || c == '-' || c == ':' || c.is_whitespace())
}

/// Splits a table row into cells, trimming outer `|` and splitting on inner `|`.
fn split_table_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let inner = trimmed
        .strip_prefix('|')
        .and_then(|s| s.strip_suffix('|'))
        .unwrap_or(trimmed);
    let mut cells: Vec<String> = inner.split('|').map(|s| s.to_string()).collect();
    // Strip leading empty cell (from a leading | with no preceding content).
    if cells.first().is_some_and(|s| s.is_empty()) {
        cells.remove(0);
    }
    // Strip trailing empty cell (from a trailing | with no following content).
    if cells.last().is_some_and(|s| s.is_empty()) {
        cells.pop();
    }
    cells
}

fn list_marker_info(line: &str) -> Option<(bool, usize, usize)> {
    let indent = leading_spaces(line);
    let t = line.trim_start();
    if t.starts_with("- ") || t.starts_with("* ") {
        Some((false, indent, 2))
    } else {
        // Ordered list: digits followed by ". " (e.g. "1. ", "42. ")
        // Use peek to avoid consuming the '.' by take_while.
        let mut chars = t.chars().peekable();
        if !chars.peek()?.is_ascii_digit() {
            return None;
        }
        let mut digit_count = 0usize;
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                chars.next();
                digit_count += 1;
            } else {
                break;
            }
        }
        if chars.next() == Some('.') && chars.next() == Some(' ') {
            Some((true, indent, digit_count + 2))
        } else {
            None
        }
    }
}

fn leading_spaces(s: &str) -> usize {
    s.chars().take_while(|&c| c == ' ').count()
}

fn take_paragraph(text: &str) -> BlockResult {
    let mut consumed = 0usize;
    let mut lines: Vec<String> = Vec::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            break;
        }
        // Heuristic: if a line looks like a block start, end the paragraph.
        let t = line.trim_start();
        if !lines.is_empty()
            && (t.starts_with("#")
                || t.starts_with("```")
                || t.starts_with("~~~")
                || t.starts_with("> ")
                || t.starts_with("- ")
                || t.starts_with("* ")
                || is_ordered_list_marker(t))
        {
            break;
        }
        lines.push(line.to_string());
        consumed += line.len() + 1;
    }

    consumed = consumed.min(text.len());

    BlockResult {
        consumed,
        lines: lines.into_iter().map(Line::raw).collect(),
    }
}

fn is_ordered_list_marker(t: &str) -> bool {
    let mut chars = t.chars().peekable();
    if chars.peek().is_some_and(|c| c.is_ascii_digit()) {
        while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
            chars.next();
        }
        chars.next() == Some('.') && chars.next() == Some(' ')
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Inline parser
// ---------------------------------------------------------------------------

/// Parses inline Markdown formatting within a single line and returns styled spans.
fn parse_inline(text: &str) -> Line<'static> {
    parse_inline_with_style(text, Style::default())
}

fn parse_inline_with_style(text: &str, base: Style) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '*' if chars.peek() == Some(&'*') => {
                flush(&mut buf, base, &mut spans);
                chars.next(); // consume second '*'
                let (inner, found) = collect_until(&mut chars, "**");
                if found {
                    let inner_spans =
                        parse_inline_with_style(&inner, base.add_modifier(Modifier::BOLD));
                    spans.extend(inner_spans.spans);
                } else {
                    // Unclosed ** — render literally
                    buf.push_str("**");
                    buf.push_str(&inner);
                }
            }
            '*' => {
                flush(&mut buf, base, &mut spans);
                let (inner, found) = collect_until_single(&mut chars, '*');
                if found {
                    let inner_spans =
                        parse_inline_with_style(&inner, base.add_modifier(Modifier::ITALIC));
                    spans.extend(inner_spans.spans);
                } else {
                    // Unclosed * — render literally
                    buf.push('*');
                    buf.push_str(&inner);
                }
            }
            '`' => {
                flush(&mut buf, base, &mut spans);
                // Count backticks for multi-backtick code spans
                let mut backtick_count = 1;
                while chars.peek() == Some(&'`') {
                    chars.next();
                    backtick_count += 1;
                }
                let (inner, found) = collect_until_backticks(&mut chars, backtick_count);
                if found {
                    let code_style = Style::default()
                        .bg(Color::Rgb(40, 40, 40))
                        .fg(Color::Rgb(255, 220, 100));
                    spans.push(Span::styled(inner, code_style));
                } else {
                    // Unclosed backticks — render literally
                    let opened: String = "`".repeat(backtick_count);
                    buf.push_str(&opened);
                    buf.push_str(&inner);
                }
            }
            '!' if chars.peek() == Some(&'[') => {
                // Image: ![alt](url)
                flush(&mut buf, base, &mut spans);
                chars.next(); // consume '['
                let (alt, _found) = collect_until_single(&mut chars, ']');
                if chars.peek() == Some(&'(') {
                    chars.next(); // consume '('
                    let (_url, _) = collect_until_single(&mut chars, ')');
                    spans.push(Span::styled(
                        format!(" 🖼 {alt} "),
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
                    ));
                } else {
                    buf.push_str("![");
                    buf.push_str(&alt);
                }
            }
            '[' => {
                flush(&mut buf, base, &mut spans);
                let (link_text, _found) = collect_until_single(&mut chars, ']');
                if chars.peek() == Some(&'(') {
                    chars.next(); // consume '('
                    let (url, _) = collect_until_single(&mut chars, ')');
                    let link_style = Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::UNDERLINED);
                    spans.push(Span::styled(link_text, link_style));
                    // Store the URL as a hidden tooltip-like suffix is not possible in ratatui,
                    // so we just render the link text styled.
                    let _ = url; // URL discarded for display; kept for future extensibility.
                } else {
                    buf.push('[');
                    buf.push_str(&link_text);
                }
            }
            '~' if chars.peek() == Some(&'~') => {
                flush(&mut buf, base, &mut spans);
                chars.next(); // consume second '~'
                let (inner, found) = collect_until(&mut chars, "~~");
                if found {
                    let strike_style = base.add_modifier(Modifier::CROSSED_OUT);
                    let inner_spans = parse_inline_with_style(&inner, strike_style);
                    spans.extend(inner_spans.spans);
                } else {
                    // Unclosed ~~ — render literally
                    buf.push_str("~~");
                    buf.push_str(&inner);
                }
            }
            '\\' => {
                // Simple escape: consume next char literally.
                if let Some(next) = chars.next() {
                    buf.push(next);
                } else {
                    buf.push('\\');
                }
            }
            _ => buf.push(ch),
        }
    }

    flush(&mut buf, base, &mut spans);
    Line::from(spans)
}

fn flush(buf: &mut String, style: Style, spans: &mut Vec<Span<'static>>) {
    if !buf.is_empty() {
        spans.push(Span::styled(buf.clone(), style));
        buf.clear();
    }
}

fn collect_until(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    delim: &str,
) -> (String, bool) {
    let mut out = String::new();
    let dchars: Vec<char> = delim.chars().collect();
    if dchars.is_empty() {
        return (out, false);
    }
    'outer: while let Some(ch) = chars.next() {
        if ch == dchars[0] {
            for (i, &dc) in dchars.iter().enumerate().skip(1) {
                if chars.peek() == Some(&dc) {
                    chars.next();
                } else {
                    // Put back consumed chars (simplification: just emit them).
                    out.push(ch);
                    for item in dchars.iter().take(i).skip(1) {
                        out.push(*item);
                    }
                    continue 'outer;
                }
            }
            return (out, true);
        }
        out.push(ch);
    }
    (out, false)
}

fn collect_until_single(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    delim: char,
) -> (String, bool) {
    let mut out = String::new();
    for ch in chars.by_ref() {
        if ch == delim {
            return (out, true);
        }
        out.push(ch);
    }
    (out, false)
}

/// Collect characters until a run of `count` backticks is found.
fn collect_until_backticks(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    count: usize,
) -> (String, bool) {
    let mut out = String::new();
    let mut buf: Vec<char> = Vec::new();
    for ch in chars.by_ref() {
        if ch == '`' {
            buf.push(ch);
            if buf.len() == count {
                return (out, true);
            }
        } else {
            out.extend(buf.drain(..));
            buf.clear();
            out.push(ch);
        }
    }
    // If closing backticks not found, include what was buffered
    out.extend(buf);
    (out, false)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let lines = render_markdown("");
        assert!(lines.is_empty());
    }

    #[test]
    fn test_plain_paragraph() {
        let lines = render_markdown("Hello world");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "Hello world");
    }

    #[test]
    fn test_bold() {
        let lines = render_markdown("**bold** text");
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert_eq!(spans[0].content, "bold");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].content, " text");
    }

    #[test]
    fn test_italic() {
        let lines = render_markdown("*italic* text");
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert_eq!(spans[0].content, "italic");
        assert_eq!(spans[1].content, " text");
    }

    #[test]
    fn test_inline_code() {
        let lines = render_markdown("`code` here");
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert_eq!(spans[0].content, "code");
        assert_eq!(spans[1].content, " here");
    }

    #[test]
    fn test_heading() {
        let lines = render_markdown("# Heading 1\n\npara");
        assert_eq!(lines[0].spans[0].content, "Heading 1");
    }

    #[test]
    fn test_blockquote() {
        let lines = render_markdown("> quote");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.starts_with("│"));
    }

    #[test]
    fn test_bullet_list() {
        let lines = render_markdown("- item 1\n- item 2");
        assert_eq!(lines.len(), 2);
        assert!(lines[0].spans[0].content.starts_with("•"));
    }

    #[test]
    fn test_ordered_list() {
        let lines = render_markdown("1. first\n2. second");
        assert_eq!(lines.len(), 2);
        assert!(lines[0].spans[0].content.starts_with("1."));
    }

    #[test]
    fn test_fenced_code_block() {
        let text = "```\nhello\nworld\n```";
        let lines = render_markdown(text);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "hello");
        assert_eq!(lines[1].spans[0].content, "world");
        // Verify line style has background (covers entire line).
        assert_eq!(lines[0].style.bg, Some(Color::Rgb(30, 30, 30)));
        assert_eq!(lines[1].style.bg, Some(Color::Rgb(30, 30, 30)));
    }

    #[test]
    fn test_link() {
        let lines = render_markdown("[text](url)");
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert_eq!(spans[0].content, "text");
    }

    #[test]
    fn test_strikethrough() {
        let lines = render_markdown("~~deleted~~");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "deleted");
    }

    #[test]
    fn test_table() {
        let text = "| Name | Age |\n|------|-----|\n| Alice | 30 |\n| Bob | 25 |";
        let lines = render_markdown(text);
        // Header + separator + 2 data rows = 4 lines
        assert_eq!(lines.len(), 4);
        // Header should be bold
        assert!(
            lines[0]
                .spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::BOLD))
        );
        // Separator should contain dashes
        assert!(lines[1].spans.iter().any(|s| s.content.contains('-')));
        // Data rows should exist
        assert!(lines[2].spans.iter().any(|s| s.content.contains("Alice")));
        assert!(lines[3].spans.iter().any(|s| s.content.contains("Bob")));
    }

    #[test]
    fn test_multiple_blocks() {
        let text = "# Title\n\nParagraph with **bold**.\n\n- list item\n\n> quote";
        let lines = render_markdown(text);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_multiple_headings() {
        let lines = render_markdown("# Title\n\n## Subtitle\n\n### Subsubtitle\n\nParagraph here.");
        assert_eq!(
            lines.len(),
            7,
            "got {} lines: {:?}",
            lines.len(),
            lines
                .iter()
                .map(|l| l
                    .spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>())
                .collect::<Vec<_>>()
        );
        assert_eq!(lines[0].spans[0].content, "Title", "first heading");
        assert_eq!(lines[2].spans[0].content, "Subtitle", "second heading");
        assert_eq!(lines[4].spans[0].content, "Subsubtitle", "third heading");
        assert_eq!(lines[6].spans[0].content, "Paragraph here.");
    }

    #[test]
    fn test_heading_no_trailing_newline() {
        let lines = render_markdown("## Hello\n\n### World");
        assert_eq!(lines.len(), 3, "got {} lines", lines.len());
        assert_eq!(lines[0].spans[0].content, "Hello");
        assert_eq!(lines[2].spans[0].content, "World");
    }

    #[test]
    fn test_heading_without_space() {
        let lines = render_markdown("###World");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "###World");
    }

    // ── Comprehensive block-element tests ──────────────────────────

    #[test]
    fn test_code_block_with_language() {
        let text = "```rust\nfn main() {}\n```";
        let lines = render_markdown(text);
        // Language label + 1 code line
        assert_eq!(lines.len(), 2, "got {} lines", lines.len());
        assert_eq!(lines[0].spans[0].content, " rust ", "language label");
        assert_eq!(lines[1].spans[0].content, "fn main() {}", "code line");
        assert_eq!(lines[0].style.bg, Some(Color::Rgb(30, 30, 30)));
        assert_eq!(lines[1].style.bg, Some(Color::Rgb(30, 30, 30)));
    }

    #[test]
    fn test_code_block_tilde_fence() {
        let text = "~~~\ncode here\n~~~";
        let lines = render_markdown(text);
        assert_eq!(lines.len(), 1, "got {} lines", lines.len());
        assert_eq!(lines[0].spans[0].content, "code here");
    }

    #[test]
    fn test_horizontal_rules() {
        for rule in &["---", "***", "___", "-----", "*****", "_____"] {
            let lines = render_markdown(rule);
            assert_eq!(
                lines.len(),
                1,
                "rule {rule:?} produced {} lines",
                lines.len()
            );
            assert_eq!(lines[0].spans[0].content, "─".repeat(50));
        }
    }

    #[test]
    fn test_horizontal_rule_too_short() {
        // Only 2 chars → not a horizontal rule
        let lines = render_markdown("--");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "--");
    }

    #[test]
    fn test_blockquote_multi_line() {
        let text = "> line one\n> line two\n> line three";
        let lines = render_markdown(text);
        assert_eq!(lines.len(), 3);
        for (i, line) in lines.iter().enumerate() {
            assert!(
                line.spans[0].content.starts_with('│'),
                "line {i} missing │ prefix"
            );
            assert!(
                line.spans[1]
                    .content
                    .contains(&format!("line {}", ["one", "two", "three"][i]))
            );
        }
    }

    #[test]
    fn test_blockquote_with_blank_line() {
        let text = "> para1\n>\n> para2";
        let lines = render_markdown(text);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].spans[1].content.contains("para1"));
        // Blank quoted line has only the │ prefix with no content span
        assert_eq!(
            lines[1].spans.len(),
            1,
            "blank quote line has only │ prefix"
        );
        assert!(lines[2].spans[1].content.contains("para2"));
    }

    // ── List tests ────────────────────────────────────────────────

    #[test]
    fn test_list_star_marker() {
        let lines = render_markdown("* item 1\n* item 2");
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].spans[0].content.starts_with('•'),
            "got {:#?}",
            lines[0].spans[0].content
        );
        assert!(lines[1].spans[0].content.starts_with('•'));
    }

    #[test]
    fn test_task_list_checked() {
        let lines = render_markdown("- [x] done\n- [X] also done");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "☑ ");
        assert_eq!(lines[1].spans[0].content, "☑ ");
    }

    #[test]
    fn test_task_list_unchecked() {
        let lines = render_markdown("- [ ] todo");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "☐ ");
    }

    #[test]
    fn test_task_list_star_marker() {
        // `* [x]` is recognized as a task list (same as `- [x]`)
        let lines = render_markdown("* [x] star task");
        assert_eq!(lines[0].spans[0].content, "☑ ");
    }

    #[test]
    fn test_list_item_with_continuation() {
        let text = "- item one\n  continuation\n- item two";
        let lines = render_markdown(text);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].spans[0].content.starts_with('•'));
        assert_eq!(lines[1].spans[0].content, "   ", "continuation indent");
        assert_eq!(lines[1].spans[1].content, "continuation");
        assert!(lines[2].spans[0].content.starts_with('•'));
    }

    #[test]
    fn test_ordered_list_custom_start() {
        // The renderer always re-numbers from 1; test that documented.
        let lines = render_markdown("3. third\n4. fourth");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "1. ");
        assert_eq!(lines[1].spans[0].content, "2. ");
    }

    // ── Table tests ───────────────────────────────────────────────

    #[test]
    fn test_table_single_column() {
        let text = "| H |\n|---|\n| v |";
        let lines = render_markdown(text);
        assert_eq!(lines.len(), 3, "got {} lines", lines.len());
        assert!(lines[0].spans.iter().any(|s| s.content.contains("H")));
        assert!(lines[2].spans.iter().any(|s| s.content.contains("v")));
    }

    #[test]
    fn test_table_no_outer_pipes() {
        // This is NOT recognized as a table (no leading/trailing |)
        let text = "a|b\n-|-\nc|d";
        let lines = render_markdown(text);
        // Should be rendered as plain paragraphs
        assert!(
            lines
                .iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains("a|b")))
        );
    }

    // ── Inline tests ──────────────────────────────────────────────

    #[test]
    fn test_inline_code_multi_backtick() {
        let lines = render_markdown("``co`de`` here");
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert_eq!(spans[0].content, "co`de");
        assert_eq!(spans[1].content, " here");
    }

    #[test]
    fn test_image() {
        let lines = render_markdown("![alt](url)");
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert!(
            spans[0].content.contains("alt"),
            "got {:?}",
            spans[0].content
        );
    }

    #[test]
    fn test_backslash_escape() {
        let lines = render_markdown("\\*not italic*");
        assert_eq!(lines.len(), 1);
        let all: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(all, "*not italic*");
    }

    #[test]
    fn test_backslash_escape_backslash() {
        let lines = render_markdown("\\\\backslash");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "\\backslash");
    }

    #[test]
    fn test_nested_bold_italic() {
        // Note: italic uses `*`, not `_` (only `*` is supported).
        // `***...***` (bold+italic) is not supported because the inline
        // parser's `collect_until` for `**` greedily absorbs the inner `*`.
        // Use separate `**bold** *italic*` instead.
        let lines = render_markdown("**bold** and *italic*");
        assert_eq!(lines.len(), 1);
        let all: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(all, "bold and italic");
    }

    #[test]
    fn test_unclosed_bold() {
        // Unclosed ** should render the opening ** literally
        let lines = render_markdown("**unclosed");
        assert_eq!(lines.len(), 1);
        let all: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        // Should contain "**unclosed" — the ** should not be silently consumed
        assert!(all.contains("**unclosed"), "got {all:?}");
    }

    #[test]
    fn test_unclosed_code() {
        let lines = render_markdown("`unclosed code");
        assert_eq!(lines.len(), 1);
        let all: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        // Backtick should be rendered literally
        assert!(all.contains('`'), "got {all:?}");
    }

    #[test]
    fn test_multiple_inline_formats() {
        let lines = render_markdown("**bold**, *italic*, `code`, and ~~strike~~.");
        assert_eq!(lines.len(), 1);
        let all: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(all.contains("bold"));
        assert!(all.contains("italic"));
        assert!(all.contains("code"));
        assert!(all.contains("strike"));
    }

    // ── Edge cases ────────────────────────────────────────────────

    #[test]
    fn test_think_tag_stripping() {
        let text = "before<think>internal reasoning</think>after";
        let lines = render_markdown(text);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "beforeafter");
    }

    #[test]
    fn test_think_tag_multiline() {
        let text = "hello\n<think>\nsecret\n</think>\nworld";
        let lines = render_markdown(text);
        // blank lines from stripped think block create spacing between paragraphs
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].spans[0].content, "hello");
        assert_eq!(lines[2].spans[0].content, "world");
    }

    #[test]
    fn test_heading_levels_4_5_6() {
        for level in 4..=6 {
            let md = format!("{} Heading {}", "#".repeat(level), level);
            let lines = render_markdown(&md);
            assert_eq!(lines.len(), 1, "level {level} got {} lines", lines.len());
            assert_eq!(lines[0].spans[0].content, format!("Heading {level}"));
        }
    }

    #[test]
    fn test_unicode_content() {
        let lines = render_markdown("你好 **世界**");
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert_eq!(spans[0].content, "你好 ");
        assert_eq!(spans[1].content, "世界");
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_paragraph_with_multiple_lines() {
        let text = "line one\nline two\nline three";
        let lines = render_markdown(text);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].spans[0].content, "line one");
        assert_eq!(lines[1].spans[0].content, "line two");
        assert_eq!(lines[2].spans[0].content, "line three");
    }

    #[test]
    fn test_mixed_blocks_without_blank_lines() {
        // Blocks separated by blank lines in render_markdown's source are needed
        // for proper block detection; without them, take_paragraph may grab
        // subsequent content.
        let text = "Paragraph\n- list\n> quote";
        let lines = render_markdown(text);
        // The paragraph parser should stop at `- list` before grabbing `> quote`
        assert!(
            lines
                .iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains("list")))
        );
    }

    #[test]
    fn test_render_markdown_does_not_panic_on_large_text() {
        let long = "word ".repeat(500);
        // Should not panic
        let lines = render_markdown(&long);
        assert!(!lines.is_empty());
    }
}
