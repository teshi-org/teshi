use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use teshi_core::markdown::strip_think_tags;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme;

/// Bullet glyphs for unordered lists, cycled by nesting depth.
const BULLETS: [&str; 5] = ["• ", "▪ ", "‣ ", "⭑ ", "◦ "];

/// Fallback horizontal-rule length when the caller does not supply a width.
const DEFAULT_RULE_WIDTH: usize = 50;

/// Left padding (in cells) inside fenced code blocks.
const CODE_LEFT_PAD: &str = "  ";

/// Clickable copy affordance shown on a chat code block's language row.
pub(crate) const CODE_COPY_LABEL: &str = "[copy]";

/// Prefix drawn in front of every blockquote line, repeated per nesting level.
const QUOTE_BAR: &str = "▌ ";

/// Per-level heading styles: H1-H3 use the primary color, H4-H6 step down to
/// the plain and muted foregrounds so deep headings do not compete with H1.
fn heading_style(level: usize) -> Style {
    let base = Style::default();
    match level {
        1 | 3 => base.fg(theme::PRIMARY).add_modifier(Modifier::BOLD),
        2 => base.fg(theme::PRIMARY).add_modifier(Modifier::UNDERLINED),
        4 => base
            .fg(theme::FG)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        5 => base.fg(theme::FG).add_modifier(Modifier::BOLD),
        _ => base.fg(theme::MUTED).add_modifier(Modifier::BOLD),
    }
}

/// Layout context threaded through the recursive block renderer.
#[derive(Clone, Copy)]
struct BlockCtx {
    /// Target width in cells; `0` disables full-width padding.
    width: usize,
    /// Current unordered-list nesting depth, used to pick the bullet glyph.
    list_depth: usize,
    /// Suppresses blank separator lines between blocks (list item bodies).
    compact: bool,
    /// Draw the chat-only copy affordance on code blocks.
    code_copy: bool,
}

/// Renders a Markdown text string into styled `ratatui` lines.
///
/// Supports block-level elements (headings, nested lists, blockquotes, fenced
/// code blocks, tables, and paragraphs) and inline formatting (bold, italic,
/// inline code, links, and strikethrough). Surfaces are not padded to a fixed
/// width; use [`render_markdown_width`] when the target width is known.
///
/// # Examples
///
/// ```
/// use teshi_tui::markdown::render_markdown;
/// let lines = render_markdown("Hello **world**");
/// assert_eq!(lines.len(), 1);
/// ```
pub fn render_markdown(text: &str) -> Vec<Line<'static>> {
    render_markdown_width(text, 0)
}

/// Renders Markdown like [`render_markdown`], padding full-width elements to `width`.
///
/// Code fences and blockquotes get their background extended to `width` cells
/// and horizontal rules span the full width. A `width` of `0` disables padding.
/// Lines are not wrapped; callers that need wrapping must do it afterwards and
/// should preserve each line's `style`, which carries the block background.
///
/// # Examples
///
/// ```
/// use teshi_tui::markdown::render_markdown_width;
/// let lines = render_markdown_width("---", 10);
/// assert_eq!(lines[0].width(), 10);
/// ```
pub fn render_markdown_width(text: &str, width: u16) -> Vec<Line<'static>> {
    // Strip <think>...</think> blocks (reasoning model output)
    let text = strip_think_tags(text);
    render_blocks(
        &text,
        BlockCtx {
            width: width as usize,
            list_depth: 0,
            compact: false,
            code_copy: false,
        },
    )
    .lines
}

/// One fenced code block found while rendering Markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeBlockSpan {
    /// Index of the block's top (language) row in the rendered lines.
    pub line: usize,
    /// Raw code, without the fence rows or the info string.
    pub source: String,
    /// Whether a chat copy affordance was drawn for this block.
    pub copy_label: bool,
}

struct RenderedBlocks {
    lines: Vec<Line<'static>>,
    code_blocks: Vec<CodeBlockSpan>,
}

/// Renders chat message content and reports its fenced code blocks so the
/// caller can attach copy hitboxes to the language rows.
pub(crate) fn render_markdown_width_with_code_blocks(
    text: &str,
    width: u16,
) -> (Vec<Line<'static>>, Vec<CodeBlockSpan>) {
    let text = strip_think_tags(text);
    let rendered = render_blocks(
        &text,
        BlockCtx {
            width: width as usize,
            list_depth: 0,
            compact: false,
            code_copy: true,
        },
    );
    (rendered.lines, rendered.code_blocks)
}

/// Returns the raw source of the block at the given render order index.
pub(crate) fn code_block_source(markdown: &str, index: usize) -> Option<String> {
    let text = strip_think_tags(markdown);
    render_blocks(
        &text,
        BlockCtx {
            width: 0,
            list_depth: 0,
            compact: false,
            code_copy: false,
        },
    )
    .code_blocks
    .into_iter()
    .nth(index)
    .map(|block| block.source)
}

fn render_blocks(input: &str, ctx: BlockCtx) -> RenderedBlocks {
    let mut lines = Vec::new();
    let mut code_blocks = Vec::new();
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
        if needs_sep && !ctx.compact {
            let already_blank = lines.last().is_some_and(is_blank_line);
            if !already_blank {
                lines.push(Line::default());
            }
        }

        // Detect block type by the first non-empty line.
        if let Some(block) = try_fenced_code_block(remaining, ctx) {
            pos += block.consumed;
            append_block(&mut lines, &mut code_blocks, block);
        } else if let Some(block) = try_heading(remaining) {
            pos += block.consumed;
            lines.push(block.line);
        } else if let Some(block) = try_horizontal_rule(remaining, ctx) {
            pos += block.consumed;
            lines.push(block.line);
        } else if let Some(block) = try_blockquote(remaining, ctx) {
            pos += block.consumed;
            append_block(&mut lines, &mut code_blocks, block);
        } else if let Some(block) = try_list(remaining, ctx) {
            pos += block.consumed;
            append_block(&mut lines, &mut code_blocks, block);
        } else if let Some(block) = try_table(remaining, ctx) {
            pos += block.consumed;
            append_block(&mut lines, &mut code_blocks, block);
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

    RenderedBlocks { lines, code_blocks }
}

fn append_block(
    lines: &mut Vec<Line<'static>>,
    code_blocks: &mut Vec<CodeBlockSpan>,
    block: BlockResult,
) {
    let base = lines.len();
    lines.extend(block.lines);
    for mut code_block in block.code_blocks {
        code_block.line += base;
        code_blocks.push(code_block);
    }
}

fn is_blank_line(line: &Line<'_>) -> bool {
    line.spans.iter().all(|s| s.content.is_empty()) && line.style == Style::default()
}

/// Appends trailing spaces in `style` so the line spans `width` cells.
/// A `width` of `0` (unknown) leaves the line untouched.
fn pad_to_width(line: &mut Line<'static>, width: usize, style: Style) {
    let current = line.width();
    if width > current {
        line.spans
            .push(Span::styled(" ".repeat(width - current), style));
    }
}

/// Prefixes every line with `prefix`, keeping each line's own block style.
fn prefix_lines<'a>(
    lines: Vec<Line<'static>>,
    prefix: &'a Span<'static>,
) -> impl Iterator<Item = Line<'static>> + 'a {
    lines.into_iter().map(move |line| {
        let mut spans = Vec::with_capacity(line.spans.len() + 1);
        spans.push(prefix.clone());
        spans.extend(line.spans.into_iter().filter(|s| !s.content.is_empty()));
        Line::from(spans).style(line.style)
    })
}

// ---------------------------------------------------------------------------
// Block parsers
// ---------------------------------------------------------------------------

struct BlockResult {
    consumed: usize,
    lines: Vec<Line<'static>>,
    code_blocks: Vec<CodeBlockSpan>,
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

fn try_fenced_code_block(text: &str, ctx: BlockCtx) -> Option<BlockResult> {
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

    let surface = Style::default().bg(theme::BLOCK_BG);
    let code_style = surface.fg(theme::FG);
    let surface_line = |spans: Vec<Span<'static>>| {
        let mut line = Line::from(spans).style(surface);
        pad_to_width(&mut line, ctx.width, surface);
        line
    };
    let mut out: Vec<Line<'static>> = Vec::with_capacity(code_lines.len().saturating_add(2));

    let source = code_lines.join("\n");

    // The top padding row doubles as the language label so the label never
    // costs an extra row.
    let mut top = vec![Span::styled(CODE_LEFT_PAD, surface)];
    if has_lang {
        top.push(Span::styled(lang.to_string(), surface.fg(theme::MUTED)));
    }
    // The chat layer overlays the clickable copy label after selection mapping,
    // so the label never enters the selectable text projection.
    let copy_label = ctx.code_copy
        && ctx.width > 0
        && ctx.width
            > top
                .iter()
                .map(|span| span.content.width())
                .sum::<usize>()
                .saturating_add(CODE_COPY_LABEL.width());
    out.push(surface_line(top));

    for code in code_lines {
        out.push(surface_line(vec![
            Span::styled(CODE_LEFT_PAD, surface),
            Span::styled(code, code_style),
        ]));
    }
    out.push(surface_line(vec![Span::styled(CODE_LEFT_PAD, surface)]));

    Some(BlockResult {
        consumed,
        lines: out,
        code_blocks: vec![CodeBlockSpan {
            line: 0,
            source,
            copy_label,
        }],
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
    let line = parse_inline_with_style(content, heading_style(level));
    Some(SingleLineResult {
        consumed: first_line.len() + 1,
        line,
    })
}

/// Detects and renders a horizontal rule: `---`, `***`, or `___` (3+ characters).
fn try_horizontal_rule(text: &str, ctx: BlockCtx) -> Option<SingleLineResult> {
    let line = text.lines().next()?;
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return None;
    }
    let ch = trimmed.chars().next()?;
    if (ch == '-' || ch == '*' || ch == '_') && trimmed.chars().all(|c| c == ch) {
        let len = if ctx.width > 0 {
            ctx.width
        } else {
            DEFAULT_RULE_WIDTH
        };
        Some(SingleLineResult {
            consumed: line.len() + 1,
            line: Line::styled("─".repeat(len), Style::default().fg(theme::SECONDARY)),
        })
    } else {
        None
    }
}

fn try_blockquote(text: &str, ctx: BlockCtx) -> Option<BlockResult> {
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

    // Quote bodies are full Markdown, so nested quotes, lists, and code render
    // recursively; nested bars accumulate one `▌` per level.
    let surface = Style::default().bg(theme::BLOCK_BG);
    let bar = Span::styled(QUOTE_BAR, surface.fg(theme::PRIMARY_MUTED));
    let inner_ctx = BlockCtx {
        width: ctx.width.saturating_sub(QUOTE_BAR.width()),
        ..ctx
    };
    let inner = render_blocks(&raw_lines.join("\n"), inner_ctx);
    let code_blocks = inner.code_blocks;
    let lines = prefix_lines(inner.lines, &bar)
        .map(|line| {
            let style = surface.patch(line.style);
            let mut line = line.style(style);
            pad_to_width(&mut line, ctx.width, style);
            line
        })
        .collect();

    Some(BlockResult {
        consumed,
        lines,
        code_blocks,
    })
}

fn try_list(text: &str, ctx: BlockCtx) -> Option<BlockResult> {
    let first_line = text.lines().next()?;
    let (is_ordered, indent, marker_len) = list_marker_info(first_line)?;

    let mut consumed = 0usize;
    let mut items: Vec<Vec<String>> = Vec::new();
    let mut current_item: Vec<String> = Vec::new();

    let mut prev_blank = false;
    for line in text.lines() {
        let line_indent = leading_spaces(line);
        let t = line.trim_start();

        if line.trim().is_empty() {
            current_item.push(String::new());
            consumed += line.len() + 1;
            prev_blank = true;
            continue;
        }

        if line_indent < indent && !t.is_empty() {
            break;
        }

        let same_level_marker = if line_indent == indent {
            list_marker_info(line).filter(|(ordered, _, _)| *ordered == is_ordered)
        } else {
            None
        };
        // An unindented, non-marker line after a blank line starts a new block
        // (e.g. a closing paragraph) rather than continuing the last item.
        if prev_blank && line_indent <= indent && same_level_marker.is_none() {
            break;
        }
        prev_blank = false;

        if let Some((_, _, mlen)) = same_level_marker {
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
    let mut code_blocks = Vec::new();
    for (idx, item_lines) in items.iter().enumerate() {
        let first = &item_lines[0];
        // Check for task list: "- [x] ..." or "- [ ] ..."
        let primary = Style::default().fg(theme::PRIMARY);
        let (bullet, bullet_style, first_text) = if is_ordered {
            (format!("{}. ", idx + 1), primary, first.as_str())
        } else if let Some(rest) = first
            .strip_prefix("[x] ")
            .or_else(|| first.strip_prefix("[X] "))
        {
            ("✓ ".to_string(), Style::default().fg(theme::SUCCESS), rest)
        } else if let Some(rest) = first.strip_prefix("[ ] ") {
            ("☐ ".to_string(), Style::default().fg(theme::MUTED), rest)
        } else {
            let glyph = BULLETS[ctx.list_depth % BULLETS.len()];
            (glyph.to_string(), primary, first.as_str())
        };
        let marker_width = bullet.width();

        let mut spans = vec![Span::styled(bullet, bullet_style)];
        spans.extend(parse_inline(first_text).spans);
        lines.push(Line::from(spans));

        // Continuation lines (including nested lists) render recursively and
        // hang under the item text, so wrapped bodies align with the marker.
        let body = item_lines[1..].join("\n");
        if !body.trim().is_empty() {
            let child_ctx = BlockCtx {
                width: ctx.width.saturating_sub(marker_width),
                list_depth: ctx.list_depth + 1,
                compact: true,
                code_copy: ctx.code_copy,
            };
            let indent = Span::raw(" ".repeat(marker_width));
            let inner = render_blocks(&body, child_ctx);
            let base = lines.len();
            lines.extend(prefix_lines(inner.lines, &indent));
            for mut code_block in inner.code_blocks {
                code_block.line += base;
                code_blocks.push(code_block);
            }
        }
    }

    Some(BlockResult {
        consumed,
        lines,
        code_blocks,
    })
}

fn try_table(text: &str, ctx: BlockCtx) -> Option<BlockResult> {
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

    // Cells are measured on the inline-rendered spans, not on the raw source:
    // `**bold**`, `` `code` `` and `[text](url)` lose their markers once
    // rendered, so measuring the source width would leave those rows short and
    // push their column borders out of line with the rest of the table.
    let cells: Vec<Vec<Line<'static>>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| parse_inline_with_style(cell.trim(), Style::default()))
                .collect()
        })
        .collect();

    let col_count = rows[0].len();
    let mut col_widths: Vec<usize> = vec![0; col_count];
    let mut col_floors: Vec<usize> = vec![1; col_count];
    for row in &cells {
        for (i, cell) in row.iter().enumerate() {
            col_widths[i] = col_widths[i].max(cell.width());
            col_floors[i] = col_floors[i].max(widest_glyph(cell));
        }
    }
    for (width, floor) in col_widths.iter_mut().zip(&col_floors) {
        *width = (*width).max(*floor);
    }

    let natural_total = col_widths.iter().map(|width| width + 2).sum::<usize>() + col_count + 1;
    if ctx.width > 0 && natural_total > ctx.width {
        let mut block = render_compact_table(cells, &col_widths, &col_floors, ctx.width);
        block.consumed = consumed;
        return Some(block);
    }

    let header_style = Style::default()
        .fg(theme::PRIMARY)
        .add_modifier(Modifier::BOLD);
    let border_style = Style::default()
        .fg(theme::BORDER)
        .add_modifier(Modifier::DIM);

    // Horizontal rule row such as `┌───┬───┐`; each column gets 1 cell of
    // padding on both sides of its content.
    let rule = |left: &str, mid: &str, right: &str| {
        let body = col_widths
            .iter()
            .map(|w| "─".repeat(w + 2))
            .collect::<Vec<_>>()
            .join(mid);
        Line::styled(format!("{left}{body}{right}"), border_style)
    };

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(rows.len() + 2);
    lines.push(rule("┌", "┬", "┐"));

    for (ri, row) in cells.iter().enumerate() {
        if ri == 1 {
            lines.push(rule("├", "┼", "┤"));
            continue;
        }

        let base_style = if ri == 0 {
            header_style
        } else {
            Style::default()
        };
        let mut spans: Vec<Span<'static>> = vec![Span::styled("│", border_style)];
        for (ci, cell) in row.iter().enumerate() {
            let pad = col_widths[ci].saturating_sub(cell.width());
            spans.push(Span::styled(" ", base_style));
            spans.extend(
                cell.spans
                    .iter()
                    .map(|span| Span::styled(span.content.clone(), base_style.patch(span.style))),
            );
            spans.push(Span::styled(" ".repeat(pad + 1), base_style));
            spans.push(Span::styled("│", border_style));
        }
        lines.push(Line::from(spans));
    }
    lines.push(rule("└", "┴", "┘"));

    Some(BlockResult {
        consumed,
        lines,
        code_blocks: Vec::new(),
    })
}

/// Renders a table that cannot keep its natural width: cells wrap inside the
/// available columns, and extremely narrow panes fall back to a borderless list.
fn render_compact_table(
    rows: Vec<Vec<Line<'static>>>,
    natural: &[usize],
    floors: &[usize],
    limit: usize,
) -> BlockResult {
    let columns = natural.len();
    // Below three content cells per column the box is unreadable; list mode is
    // the honest fallback.
    let min_total = columns * 6 + 1;
    if limit < min_total {
        return table_list_fallback(&rows);
    }

    let budget = limit - (columns * 3 + 1);
    let mut widths = natural.to_vec();
    while widths.iter().sum::<usize>() > budget {
        // Never squeeze a column below the widest glyph it must hold: a 2-cell
        // CJK glyph in a 1-cell column makes that row's borders drift.
        let Some(widest) = widths
            .iter()
            .enumerate()
            .filter(|(index, width)| **width > floors[*index])
            .max_by_key(|(_, width)| **width)
            .map(|(index, _)| index)
        else {
            return table_list_fallback(&rows);
        };
        widths[widest] -= 1;
    }

    let border_style = Style::default()
        .fg(theme::BORDER)
        .add_modifier(Modifier::DIM);
    let header_style = Style::default()
        .fg(theme::PRIMARY)
        .add_modifier(Modifier::BOLD);
    let rule = |left: &str, mid: &str, right: &str| {
        let body = widths
            .iter()
            .map(|width| "─".repeat(width + 2))
            .collect::<Vec<_>>()
            .join(mid);
        Line::styled(format!("{left}{body}{right}"), border_style)
    };

    let mut lines = Vec::new();
    lines.push(rule("┌", "┬", "┐"));
    for (row_index, row) in rows.iter().enumerate() {
        if row_index == 1 {
            lines.push(rule("├", "┼", "┤"));
            continue;
        }
        let base_style = if row_index == 0 {
            header_style
        } else {
            Style::default()
        };
        let wrapped: Vec<Vec<Vec<Span<'static>>>> = row
            .iter()
            .enumerate()
            .map(|(column, cell)| wrap_table_cell(cell, widths[column], base_style))
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for line_index in 0..height {
            let mut spans = vec![Span::styled("│", border_style)];
            for column in 0..columns {
                let content = wrapped[column].get(line_index);
                let used = content
                    .map(|spans| spans.iter().map(|span| span.content.width()).sum::<usize>())
                    .unwrap_or(0);
                spans.push(Span::raw(" "));
                if let Some(content) = content {
                    spans.extend(content.iter().cloned());
                }
                spans.push(Span::raw(
                    " ".repeat(widths[column].saturating_sub(used) + 1),
                ));
                spans.push(Span::styled("│", border_style));
            }
            lines.push(Line::from(spans));
        }
    }
    lines.push(rule("└", "┴", "┘"));

    BlockResult {
        consumed: 0,
        lines,
        code_blocks: Vec::new(),
    }
}

/// Wraps one cell to a column width while keeping inline span styles.
fn wrap_table_cell(cell: &Line<'static>, width: usize, base: Style) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let cells: Vec<(char, Style)> = cell
        .spans
        .iter()
        .flat_map(|span| {
            let style = base.patch(span.style);
            span.content
                .chars()
                .map(move |character| (character, style))
        })
        .collect();
    if cells.is_empty() {
        return vec![Vec::new()];
    }

    let mut remaining = cells;
    let mut rows = Vec::new();
    while !remaining.is_empty() {
        let mut used = 0usize;
        let mut take = 0usize;
        let mut last_space = None;
        for (index, (character, _)) in remaining.iter().enumerate() {
            let character_width = character.width().unwrap_or(0);
            if take > 0 && used + character_width > width {
                break;
            }
            used += character_width;
            take = index + 1;
            if character.is_whitespace() {
                last_space = Some(take);
            }
            if used >= width {
                break;
            }
        }
        if take == 0 {
            take = 1;
        }
        let split = last_space
            .filter(|split| *split > 0 && *split < take)
            .unwrap_or(take);
        let rest = remaining.split_off(split);
        rows.push(table_spans(remaining));
        remaining = rest;
    }
    rows
}

fn table_spans(cells: Vec<(char, Style)>) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut buffer = String::new();
    let mut current: Option<Style> = None;
    for (character, style) in cells {
        if current.is_some_and(|current| current != style) {
            spans.push(Span::styled(
                std::mem::take(&mut buffer),
                current.unwrap_or_default(),
            ));
        }
        current = Some(style);
        buffer.push(character);
    }
    if !buffer.is_empty() {
        spans.push(Span::styled(buffer, current.unwrap_or_default()));
    }
    spans
}

/// Width of the widest single glyph in a rendered cell.
///
/// A column narrower than this cannot render its content without overflowing
/// the cell, which would shift that row's borders.
fn widest_glyph(cell: &Line<'static>) -> usize {
    cell.spans
        .iter()
        .flat_map(|span| span.content.chars())
        .map(|character| character.width().unwrap_or(0))
        .max()
        .unwrap_or(0)
        .max(1)
}

fn table_list_fallback(rows: &[Vec<Line<'static>>]) -> BlockResult {
    let text = |line: &Line<'static>| -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    };
    let headers = rows.first().cloned().unwrap_or_default();
    let mut lines = Vec::new();
    for row in rows.iter().skip(2) {
        for (column, cell) in row.iter().enumerate() {
            let label = headers.get(column).map(text).unwrap_or_default();
            let body = text(cell);
            let text = if label.is_empty() {
                body
            } else {
                format!("{label}: {body}")
            };
            lines.push(parse_inline_with_style(&text, Style::default()));
        }
        lines.push(Line::default());
    }
    if lines.is_empty() {
        for header in &headers {
            lines.push(parse_inline_with_style(&text(header), Style::default()));
        }
    } else {
        lines.pop();
    }
    BlockResult {
        consumed: 0,
        lines,
        code_blocks: Vec::new(),
    }
}

/// Checks whether a line is a table separator (only |, -, :, and spaces).
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
        code_blocks: Vec::new(),
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
                    let code_style = base.fg(theme::WARNING);
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
                        Style::default().fg(theme::MUTED),
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
                    let link_style = base.fg(theme::PRIMARY).add_modifier(Modifier::UNDERLINED);
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
        assert!(lines[0].spans[0].content.starts_with('▌'));
        assert_eq!(lines[0].style.bg, Some(theme::BLOCK_BG));
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
        // Top padding + 2 code lines + bottom padding.
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[1].spans[1].content, "hello");
        assert_eq!(lines[2].spans[1].content, "world");
        for (i, line) in lines.iter().enumerate() {
            assert_eq!(
                line.style.bg,
                Some(theme::BLOCK_BG),
                "line {i} missing block background"
            );
        }
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
        // Top rule + header + separator + 2 data rows + bottom rule = 6 lines
        assert_eq!(lines.len(), 6);
        assert!(lines[0].spans[0].content.starts_with('┌'));
        // Header should be bold
        assert!(
            lines[1]
                .spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::BOLD))
        );
        assert!(lines[2].spans[0].content.starts_with('├'));
        assert!(lines[3].spans.iter().any(|s| s.content.contains("Alice")));
        assert!(lines[4].spans.iter().any(|s| s.content.contains("Bob")));
        assert!(lines[5].spans[0].content.starts_with('└'));
        // Every row must have the same display width so the box lines up.
        let widths: Vec<usize> = lines.iter().map(Line::width).collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "got {widths:?}");
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
        // Label lives in the top padding row: label row + 1 code line + bottom padding.
        assert_eq!(lines.len(), 3, "got {} lines", lines.len());
        assert_eq!(lines[0].spans[1].content, "rust", "language label");
        assert_eq!(lines[0].spans[1].style.fg, Some(theme::MUTED));
        assert_eq!(lines[1].spans[1].content, "fn main() {}", "code line");
        assert_eq!(lines[0].style.bg, Some(theme::BLOCK_BG));
        assert_eq!(lines[1].style.bg, Some(theme::BLOCK_BG));
    }

    #[test]
    fn test_code_block_tilde_fence() {
        let text = "~~~\ncode here\n~~~";
        let lines = render_markdown(text);
        assert_eq!(lines.len(), 3, "got {} lines", lines.len());
        assert_eq!(lines[1].spans[1].content, "code here");
    }

    #[test]
    fn code_block_render_reports_exact_source_and_copy_label() {
        let text = "~~~rust\nfn main() {\n    println!(\"hi\");\n}\n~~~\n\n~~~sh\necho hi\n~~~";
        let (lines, blocks) = render_markdown_width_with_code_blocks(text, 40);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].source, "fn main() {\n    println!(\"hi\");\n}");
        assert_eq!(blocks[1].source, "echo hi");
        assert!(blocks.iter().all(|block| block.copy_label));
        for block in &blocks {
            assert!(
                lines[block.line]
                    .spans
                    .iter()
                    .all(|span| span.content != CODE_COPY_LABEL),
                "the chat layer owns the copy label"
            );
        }
        assert_eq!(code_block_source(text, 1).as_deref(), Some("echo hi"));
        assert_eq!(code_block_source(text, 2), None);
    }

    #[test]
    fn long_code_block_source_copies_beyond_the_viewport() {
        let code = (0..500)
            .map(|line| format!("let value_{line} = {line};"))
            .collect::<Vec<_>>()
            .join("\n");
        let text = format!("~~~rust\n{code}\n~~~");
        assert_eq!(code_block_source(&text, 0), Some(code));
    }

    #[test]
    fn plain_markdown_titles_have_no_copy_label() {
        let lines = render_markdown_width("~~~rust\nx\n~~~", 40);
        assert!(
            lines[0]
                .spans
                .iter()
                .all(|span| span.content != CODE_COPY_LABEL)
        );
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
                line.spans[0].content.starts_with('▌'),
                "line {i} missing ▌ prefix"
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
            "blank quote line has only ▌ prefix"
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
        assert_eq!(lines[0].spans[0].content, "✓ ");
        assert_eq!(lines[1].spans[0].content, "✓ ");
        assert_eq!(lines[0].spans[0].style.fg, Some(theme::SUCCESS));
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
        assert_eq!(lines[0].spans[0].content, "✓ ");
    }

    #[test]
    fn test_list_item_with_continuation() {
        let text = "- item one\n  continuation\n- item two";
        let lines = render_markdown(text);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].spans[0].content.starts_with('•'));
        assert_eq!(
            lines[1].spans[0].content, "  ",
            "continuation hangs under item text"
        );
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

    /// Cell offsets of every `│` in a rendered row.
    fn border_columns(row: &str) -> Vec<usize> {
        let mut columns = Vec::new();
        let mut width = 0;
        for character in row.chars() {
            if character == '│' {
                columns.push(width);
            }
            width += character.width().unwrap_or(0);
        }
        columns
    }

    #[test]
    fn inline_markup_does_not_shift_table_borders() {
        // `**`, backticks and link targets render to fewer cells than their
        // source, so measuring the source used to shorten those rows and drag
        // their column borders left.
        let text = "\
| 名称 | 说明 |
|---|---|
| plain | 普通文本 |
| bold | **粗体** |
| code | `code` |
| link | [文字](https://example.com/a/b/c) |";
        let lines = render_markdown_width(text, 80);
        let rows: Vec<String> = lines.iter().map(ToString::to_string).collect();
        let reference = border_columns(&rows[1]);
        for row in rows.iter().filter(|row| row.starts_with('│')) {
            assert_eq!(border_columns(row), reference, "row: {row}");
        }
        for rule in rows.iter().filter(|row| row.starts_with(['┌', '├', '└'])) {
            assert_eq!(
                UnicodeWidthStr::width(rule.as_str()),
                rows[1].width(),
                "{rule}"
            );
        }
    }

    #[test]
    fn compact_table_keeps_cjk_glyphs_whole() {
        // A column squeezed below one glyph's width used to overflow its cell
        // and push that row's right border one cell out.
        let text = "| ID | 优先级 | 需求 |\n|---|---|---|\n| doc-16 | P0 | 端到端硬超时、取消与执行隔离 |\n| doc-17 | P0 | 协议级健康检查与安全恢复 |";
        for limit in [30u16, 34, 40, 52] {
            let lines = render_markdown_width(text, limit);
            let rows: Vec<String> = lines.iter().map(ToString::to_string).collect();
            let reference = border_columns(&rows[1]);
            for row in rows.iter().filter(|row| row.starts_with('│')) {
                assert_eq!(border_columns(row), reference, "limit {limit} row: {row}");
            }
        }
    }

    #[test]
    fn wide_table_wraps_inside_cells_and_keeps_borders_aligned() {
        let text = "| 工具 | worktree 放在哪 | 初始化 | 结果怎么回到主工作区 |\n|---|---|---|---|\n| conductor | 主仓库的 .worktrees 目录 | conductor.json 里定义 setup、run、archive 三个脚本以及环境变量 | 通过 worktree 路径读取 |";
        let lines = render_markdown_width(text, 64);
        let widths: Vec<usize> = lines.iter().map(Line::width).collect();
        assert!(
            widths.iter().all(|width| *width == 64),
            "every row keeps the pane width: {widths:?}"
        );
        let rows: Vec<String> = lines.iter().map(ToString::to_string).collect();
        let borders = |row: &str| {
            let mut columns = Vec::new();
            let mut width = 0;
            for character in row.chars() {
                if character == '│' {
                    columns.push(width);
                }
                width += character.width().unwrap_or(0);
            }
            columns
        };
        let reference = borders(
            rows.iter()
                .find(|row| row.starts_with('│'))
                .expect("a content row"),
        );
        for row in rows.iter().filter(|row| row.starts_with('│')) {
            assert_eq!(borders(row), reference, "row: {row}");
        }
        assert!(rows.iter().any(|row| row.starts_with('┌')));
        assert!(rows.iter().any(|row| row.starts_with('└')));
    }

    #[test]
    fn very_narrow_table_falls_back_to_a_list() {
        let text = "| 工具 | 结果 |\n|---|---|\n| conductor | 回到主工作区 |";
        let lines = render_markdown_width(text, 10);
        let text: String = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains('│'), "{text}");
        assert!(!text.contains('┌'), "{text}");
        assert!(text.contains("工具: conductor"), "{text}");
        assert!(text.contains("结果: 回到主工作区"), "{text}");
    }

    #[test]
    fn wrapped_table_cells_keep_inline_styles() {
        let text = "| Column |\n|---|\n| **bold words that wrap** |";
        let lines = render_markdown_width(text, 12);
        assert!(lines.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content.contains("bold") && span.style.add_modifier.contains(Modifier::BOLD)
            })
        }));
    }

    #[test]
    fn test_table_single_column() {
        let text = "| H |\n|---|\n| v |";
        let lines = render_markdown(text);
        assert_eq!(lines.len(), 5, "got {} lines", lines.len());
        assert!(lines[1].spans.iter().any(|s| s.content.contains("H")));
        assert!(lines[3].spans.iter().any(|s| s.content.contains("v")));
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
    fn test_nested_list_cycles_bullets_by_depth() {
        let text = "- one\n  - two\n    - three";
        let lines = render_markdown(text);
        let rendered: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert_eq!(rendered, vec!["• one", "  ▪ two", "    ‣ three"]);
    }

    #[test]
    fn test_paragraph_after_list_is_not_continuation() {
        let lines = render_markdown("- a\n- b\n\nClosing text");
        assert_eq!(lines.len(), 4, "list, list, blank, paragraph");
        assert_eq!(lines[3].spans[0].content, "Closing text");
    }

    #[test]
    fn test_render_markdown_width_pads_full_width_blocks() {
        let cases = [
            ("```\nx\n```", "code fence"),
            ("> quoted", "blockquote"),
            ("---", "horizontal rule"),
        ];
        for (text, label) in cases {
            let lines = render_markdown_width(text, 24);
            for line in &lines {
                assert_eq!(line.width(), 24, "{label}: line not padded: {line:?}");
            }
        }
    }

    #[test]
    fn test_heading_levels_use_theme_styles() {
        let h1 = &render_markdown("# a")[0].spans[0].style;
        assert_eq!(h1.fg, Some(theme::PRIMARY));
        assert!(h1.add_modifier.contains(Modifier::BOLD));
        let h2 = &render_markdown("## a")[0].spans[0].style;
        assert!(h2.add_modifier.contains(Modifier::UNDERLINED));
        let h6 = &render_markdown("###### a")[0].spans[0].style;
        assert_eq!(h6.fg, Some(theme::MUTED));
    }

    #[test]
    fn test_render_markdown_does_not_panic_on_large_text() {
        let long = "word ".repeat(500);
        // Should not panic
        let lines = render_markdown(&long);
        assert!(!lines.is_empty());
    }
}
