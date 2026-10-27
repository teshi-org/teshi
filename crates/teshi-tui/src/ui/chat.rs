//! AI chat rendering shared by the AI tab and the MindMap side panel.
//!
//! Chat history is wrapped here rather than by `Paragraph`, so every wrapped
//! row can carry the message's colored left rail and block backgrounds can be
//! padded to the full row width.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use teshi_agent::pipeline::GenerationStage;
use teshi_core::llm::ToolCall;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::token_usage_label;
use crate::app::{
    AiChatMessage, AiRole, AiStatus, App, ChangeKind, ChatAnchor, ChatSelection, ClickableRegion,
    SLASH_COMMANDS,
};
use crate::markdown::render_markdown_width;
use crate::theme;

/// Left rail drawn on every row of a user or assistant message.
const RAIL: &str = "▌ ";
/// Indent applied to tool-result bodies, which have no rail.
const TOOL_RESULT_INDENT: &str = "    ";
/// Fallback cursor glyph appended to a streaming response.
const STREAM_CURSOR: &str = "▌";

/// Where the chat panel is drawn; selects greeting length and optional extras.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ChatPlacement {
    /// Full-size panel in the AI tab, with slash-command suggestions.
    Tab,
    /// Compact panel embedded in the MindMap tab.
    Embedded,
}

#[derive(Clone)]
struct ChatSourceRange {
    block: usize,
    start: usize,
    end: usize,
    display_start: usize,
    display_end: usize,
}

#[derive(Clone)]
struct ChatProjectedLine {
    line: Line<'static>,
    source: Option<ChatSourceRange>,
}

struct ChatBlock {
    key: usize,
    text: String,
}

struct ChatProjection {
    lines: Vec<ChatProjectedLine>,
    blocks: Vec<ChatBlock>,
}

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

fn fingerprint(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn plain_markdown_text(text: &str) -> String {
    render_markdown_width(text, 0)
        .iter()
        .map(line_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn find_char_text(haystack: &str, needle: &str, from: usize) -> Option<(usize, usize)> {
    let haystack: Vec<char> = haystack.chars().collect();
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() || from > haystack.len() || needle.len() > haystack.len() {
        return None;
    }
    let last = haystack.len().saturating_sub(needle.len());
    (from..=last).find_map(|start| {
        (haystack[start..start + needle.len()] == needle[..])
            .then_some((start, start + needle.len()))
    })
}

fn find_display_source(
    block_text: &str,
    display_text: &str,
    source_cursor: usize,
    display_start: usize,
) -> Option<(usize, usize, usize)> {
    let display_chars: Vec<char> = display_text.chars().collect();
    if let Some((start, end)) = find_char_text(block_text, display_text, source_cursor) {
        return Some((start, end, display_start));
    }
    let has_display_prefix = display_chars
        .first()
        .is_some_and(|character| character.is_whitespace())
        || display_text.starts_with("Done")
        || display_text.starts_with("Failed")
        || display_text.starts_with("Running")
        || display_text.starts_with("Pending")
        || display_text.starts_with("Unconfirmed")
        || display_text.starts_with('✓')
        || display_text.starts_with('✗')
        || display_text.starts_with('○');
    if !has_display_prefix {
        return None;
    }
    for prefix in 1..display_chars.len() {
        let candidate: String = display_chars[prefix..].iter().collect();
        if let Some((start, end)) = find_char_text(block_text, &candidate, source_cursor) {
            return Some((start, end, display_start + prefix));
        }
    }
    None
}

fn append_unselectable(out: &mut Vec<ChatProjectedLine>, lines: Vec<Line<'static>>) {
    out.extend(
        lines
            .into_iter()
            .map(|line| ChatProjectedLine { line, source: None }),
    );
}

fn append_blank(out: &mut Vec<ChatProjectedLine>) {
    out.push(ChatProjectedLine {
        line: Line::default(),
        source: None,
    });
}

/// Add rendered rows and map visible body text to a width-independent block.
///
/// The block is the rendered plain-text projection of a message. Markdown
/// syntax is intentionally omitted from this projection, so selection copies
/// what the user can read rather than hidden formatting markers.
fn append_selectable_chunk(
    out: &mut Vec<ChatProjectedLine>,
    blocks: &mut Vec<ChatBlock>,
    lines: Vec<Line<'static>>,
    block_key: usize,
    block_text: String,
) {
    if !blocks.iter().any(|block| block.key == block_key) {
        blocks.push(ChatBlock {
            key: block_key,
            text: block_text.clone(),
        });
    }
    let mut source_cursor = 0;
    for line in lines {
        let full_text = line_text(&line);
        let (display_start, selectable) = if let Some(body) = full_text.strip_prefix(RAIL) {
            (RAIL.chars().count(), body)
        } else {
            (0, full_text.as_str())
        };
        let candidate = selectable.trim_end_matches(' ');
        let source = find_display_source(&block_text, candidate, source_cursor, display_start).map(
            |(start, end, display_start)| ChatSourceRange {
                block: block_key,
                start,
                end,
                display_start,
                display_end: display_start + end.saturating_sub(start),
            },
        );
        if let Some(source) = &source {
            source_cursor = source.end;
        }
        out.push(ChatProjectedLine { line, source });
    }
}

fn block_text_for_message(msg: &AiChatMessage) -> String {
    if msg.source.as_deref() == Some("acp_reasoning") {
        plain_markdown_text(msg.reasoning_content.as_deref().unwrap_or_default())
    } else {
        plain_markdown_text(&msg.content)
    }
}

fn selected_interval(
    projection: &ChatProjection,
    selection: ChatSelection,
    source: &ChatSourceRange,
) -> Option<(usize, usize)> {
    let anchor_block = projection
        .blocks
        .iter()
        .position(|block| block.key == selection.anchor.block)?;
    let focus_block = projection
        .blocks
        .iter()
        .position(|block| block.key == selection.focus.block)?;
    let (first_block, first_offset, last_block, last_offset) = if anchor_block < focus_block
        || (anchor_block == focus_block && selection.anchor.offset <= selection.focus.offset)
    {
        (
            anchor_block,
            selection.anchor.offset,
            focus_block,
            selection.focus.offset,
        )
    } else {
        (
            focus_block,
            selection.focus.offset,
            anchor_block,
            selection.anchor.offset,
        )
    };
    let source_block = projection
        .blocks
        .iter()
        .position(|block| block.key == source.block)?;
    if source_block < first_block || source_block > last_block {
        return None;
    }
    let start = if source_block == first_block {
        first_offset
    } else {
        0
    };
    let end = if source_block == last_block {
        last_offset
    } else {
        projection.blocks[source_block].text.chars().count()
    };
    let start = start.max(source.start).min(source.end);
    let end = end.max(start).min(source.end);
    (start < end).then_some((start, end))
}

fn apply_selection(line: Line<'static>, range: std::ops::Range<usize>) -> Line<'static> {
    if range.start >= range.end {
        return line;
    }
    let patch = theme::selected_row(true);
    let line_style = line.style;
    let mut out = Vec::new();
    let mut char_index = 0;
    for span in line.spans {
        let text = span.content.to_string();
        let chars: Vec<char> = text.chars().collect();
        let span_start = char_index;
        let span_end = span_start + chars.len();
        let lo = range.start.max(span_start);
        let hi = range.end.min(span_end);
        if lo >= hi {
            out.push(span);
        } else {
            let lo = lo - span_start;
            let hi = hi - span_start;
            if lo > 0 {
                out.push(Span::styled(
                    chars[..lo].iter().collect::<String>(),
                    span.style,
                ));
            }
            out.push(Span::styled(
                chars[lo..hi].iter().collect::<String>(),
                span.style.patch(patch),
            ));
            if hi < chars.len() {
                out.push(Span::styled(
                    chars[hi..].iter().collect::<String>(),
                    span.style,
                ));
            }
        }
        char_index = span_end;
    }
    Line::from(out).style(line_style)
}

impl ChatProjection {
    fn selection_valid(&self, selection: ChatSelection) -> bool {
        let Some(anchor) = self.blocks.iter().find(|b| b.key == selection.anchor.block) else {
            return false;
        };
        let Some(focus) = self.blocks.iter().find(|b| b.key == selection.focus.block) else {
            return false;
        };
        anchor.text.chars().count() >= selection.anchor.offset
            && focus.text.chars().count() >= selection.focus.offset
            && fingerprint(&anchor.text) == selection.anchor.fingerprint
            && fingerprint(&focus.text) == selection.focus.fingerprint
    }

    fn anchor_at(&self, line_index: usize, column: usize) -> Option<ChatAnchor> {
        let line = self.lines.get(line_index)?;
        let source = line.source.as_ref()?;
        let line_text = line_text(&line.line);
        let display = line_text.chars().collect::<Vec<_>>();
        let start = source.display_start.min(display.len());
        let end = source.display_end.min(display.len());
        let char_column = if column <= start {
            0
        } else {
            let mut cells = 0;
            let mut index = start;
            while index < end {
                let width = display[index].width().unwrap_or(0);
                if column < cells + width + start {
                    break;
                }
                cells += width;
                index += 1;
            }
            index.saturating_sub(start)
        };
        Some(ChatAnchor {
            block: source.block,
            offset: (source.start + char_column).min(source.end),
            fingerprint: self
                .blocks
                .iter()
                .find(|block| block.key == source.block)
                .map_or(0, |block| fingerprint(&block.text)),
        })
    }

    fn selected_text(&self, selection: ChatSelection) -> Option<String> {
        if !self.selection_valid(selection) {
            return None;
        }
        let anchor_index = self
            .blocks
            .iter()
            .position(|block| block.key == selection.anchor.block)?;
        let focus_index = self
            .blocks
            .iter()
            .position(|block| block.key == selection.focus.block)?;
        let (first_index, first_offset, last_index, last_offset) = if anchor_index < focus_index
            || (anchor_index == focus_index && selection.anchor.offset <= selection.focus.offset)
        {
            (
                anchor_index,
                selection.anchor.offset,
                focus_index,
                selection.focus.offset,
            )
        } else {
            (
                focus_index,
                selection.focus.offset,
                anchor_index,
                selection.anchor.offset,
            )
        };
        let slice = |text: &str, start: usize, end: usize| {
            text.chars()
                .skip(start)
                .take(end.saturating_sub(start))
                .collect::<String>()
        };
        if first_index == last_index {
            return Some(slice(
                &self.blocks[first_index].text,
                first_offset,
                last_offset,
            ));
        }
        let mut result = slice(
            &self.blocks[first_index].text,
            first_offset,
            self.blocks[first_index].text.chars().count(),
        );
        for block in &self.blocks[first_index + 1..last_index] {
            result.push_str("\n\n");
            result.push_str(&block.text);
        }
        result.push_str("\n\n");
        result.push_str(&slice(&self.blocks[last_index].text, 0, last_offset));
        Some(result)
    }

    fn highlighted_line(&self, index: usize, selection: Option<ChatSelection>) -> Line<'static> {
        let line = &self.lines[index];
        let Some(selection) = selection else {
            return line.line.clone();
        };
        let Some(source) = line.source.as_ref() else {
            return line.line.clone();
        };
        let Some((start, end)) = selected_interval(self, selection, source) else {
            return line.line.clone();
        };
        apply_selection(
            line.line.clone(),
            source.display_start + start.saturating_sub(source.start)
                ..source.display_start + end.saturating_sub(source.start),
        )
    }
}

fn fg(color: ratatui::style::Color) -> Style {
    Style::default().fg(color)
}

fn muted() -> Style {
    theme::muted()
}

fn selected_row() -> Style {
    theme::selected_row(true)
}

/// Frame-clock helper shared by the animated glyphs below.
fn frame_index(period_ms: u128, frames: usize) -> usize {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    (elapsed / period_ms) as usize % frames
}

/// Rotating half-circle spinner for in-flight tool calls.
fn tool_spinner_frame() -> &'static str {
    const FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];
    FRAMES[frame_index(120, FRAMES.len())]
}

/// Three-dot pulse shown while the model is thinking.
fn thinking_pulse_frame() -> &'static str {
    const FRAMES: [&str; 4] = ["●∙∙", "∙●∙", "∙∙●", "∙●∙"];
    FRAMES[frame_index(160, FRAMES.len())]
}

fn shrink_left(area: Rect, cols: u16) -> Rect {
    let cols = cols.min(area.width);
    Rect::new(area.x + cols, area.y, area.width - cols, area.height)
}

/// Renders the agent sidebar: one row per agent with a status dot.
pub(super) fn render_agent_sidebar(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    super::pointer_controls::render_agent_sidebar(frame, app, area);
}

/// Renders the full chat panel: history, status row, optional slash-command
/// suggestions, and the input box.
///
/// `focused` controls the border highlight and whether the input box is live.
pub(super) fn render_chat_panel(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    focused: bool,
    placement: ChatPlacement,
) {
    if area.width < 10 || area.height < 3 {
        return;
    }

    let mut title = Line::from(Span::styled(
        " AI Chat ",
        fg(theme::PRIMARY).add_modifier(Modifier::BOLD),
    ));
    if !app.agent_registry.uses_builtin_fallback() {
        title.push_span(Span::styled(
            format!("· Agent: {} ", app.active_agent_profile().name),
            muted(),
        ));
    }
    let block = theme::panel_block(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_approval_badge(frame, app, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let input_active = focused && app.ai_input_focused;
    let has_acp = app.agent().backend.acp_controls().is_some();
    let status_height: u16 = 1;

    // The input grows with its wrapped content but never takes more than a
    // third of the panel, so history always stays visible.
    let input_text_width = inner.width.saturating_sub(2);
    let input_text_rows = visual_line_count_for_width(&app.agent().input, input_text_width)
        .min((inner.height / 3).max(3));
    let input_height: u16 = (input_text_rows + 2).min(inner.height.saturating_sub(status_height));

    let filter = slash_filter(app, placement);
    let sugg_count = filter
        .as_deref()
        .map_or(0, |f| filtered_slash_commands(f).len() as u16);
    let sugg_height: u16 = if sugg_count > 0 {
        let max_possible = inner
            .height
            .saturating_sub(status_height + input_height + 1);
        (sugg_count + 1).min(max_possible) // +1 for the hint line
    } else {
        0
    };

    let chat_height = inner
        .height
        .saturating_sub(status_height + sugg_height + input_height);
    let chat_area = shrink_left(Rect::new(inner.x, inner.y, inner.width, chat_height), 1);
    let status_area = shrink_left(
        Rect::new(inner.x, inner.y + chat_height, inner.width, status_height),
        1,
    );
    let sugg_area = Rect::new(
        inner.x,
        inner.y + chat_height + status_height,
        inner.width,
        sugg_height,
    );
    let input_area = Rect::new(
        inner.x,
        inner.y + chat_height + status_height + sugg_height,
        inner.width,
        input_height,
    );

    render_chat_history(frame, app, chat_area, placement);
    if has_acp {
        let has_status = app.agent().status != AiStatus::Idle
            || app.agent().task_interrupted()
            || !matches!(
                app.generation_stage,
                GenerationStage::Idle | GenerationStage::Complete
            );
        // Preserve the activity first, then the agent/settings controls. Detailed
        // messages remain in the conversation and approval UI when this row clips.
        let status_width = if has_status {
            (status_area.width / 2).min(48)
        } else {
            0
        };
        if status_width > 0 {
            render_chat_status(
                frame,
                app,
                Rect::new(status_area.x, status_area.y, status_width, 1),
            );
        }
        crate::acp_controls::render_bar(
            frame,
            app,
            Rect::new(
                status_area.x + status_width,
                status_area.y,
                status_area.width - status_width,
                1,
            ),
        );
    } else {
        render_chat_status(frame, app, status_area);
    }
    if let Some(filter) = filter.as_deref()
        && sugg_height > 0
    {
        render_slash_suggestions(frame, app, sugg_area, filter);
    }
    render_chat_input(frame, app, input_area, input_active);
}

/// Draws the approval-mode chip on the panel's top border and registers it as
/// a click target that opens the approval picker.
fn render_approval_badge(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if app.agent().backend.kind() == teshi_agent::backend::AgentBackendKind::Acp {
        return;
    }
    let mode = app.effective_approval_mode();
    let label = format!(" {} ", mode.display_name());
    let chip_bg = if mode.requires_manual_approval() {
        theme::PRIMARY
    } else if mode.auto_accepts() {
        theme::WARNING
    } else {
        theme::ERROR
    };
    let width = label.width() as u16;
    // Keep one border cell plus one gap cell to the right of the chip.
    let badge_x = area.right().saturating_sub(width + 2);
    if badge_x <= area.x {
        return;
    }
    let col_right = badge_x + width;
    app.clickable_regions.push(ClickableRegion::ApprovalBadge {
        row_y: area.y,
        col_x: badge_x,
        col_right,
    });

    let mut style = theme::chip(label.as_str(), chip_bg).style;
    if app
        .mouse_position
        .is_some_and(|(mx, my)| my == area.y && mx >= badge_x && mx < col_right)
    {
        style = style.add_modifier(Modifier::REVERSED);
    }
    frame.render_widget(
        Paragraph::new(Line::styled(label, style)),
        Rect::new(badge_x, area.y, width, 1),
    );
}

/// Renders the scrolled chat history plus a thin scrollbar on the right.
fn render_chat_history(frame: &mut Frame<'_>, app: &mut App, area: Rect, placement: ChatPlacement) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let [body, scrollbar] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).areas(area);
    app.chat_history_rect = Some(body);
    let projection =
        build_chat_projection(app, body.width as usize, body.height as usize, placement);
    let raw_selection = app.chat_selection;
    let selection_is_valid = raw_selection.is_some_and(|selection| {
        selection.agent_id == app.selected_agent && projection.selection_valid(selection)
    });
    let selection = raw_selection.filter(|_| selection_is_valid);
    if app.chat_selection.is_some() && selection.is_none() {
        app.chat_selection = None;
        app.chat_selection_dragging = false;
    }

    // Lines are already wrapped, so the row count is exact. `scroll_offset`
    // counts rows up from the bottom so new output stays pinned by default.
    let total = projection.lines.len();
    let height = body.height as usize;
    let max_start = total.saturating_sub(height);
    let start = max_start.saturating_sub(app.agent().scroll_offset.min(max_start));
    let visible: Vec<Line<'static>> = (start..start.saturating_add(height).min(total))
        .map(|index| projection.highlighted_line(index, selection))
        .collect();
    frame.render_widget(Paragraph::new(Text::from(visible)), body);
    render_scrollbar(frame, scrollbar, total, height, start);
}

fn render_scrollbar(frame: &mut Frame<'_>, area: Rect, total: usize, height: usize, start: usize) {
    if total <= height || area.width == 0 || height == 0 {
        return;
    }
    let lines = theme::scrollbar_lines(total, height, start);
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

/// Builds the wrapped chat history for a viewport `width` cells wide.
///
/// `view_height` only bounds how many inline diff rows are shown.
#[cfg(test)]
fn build_chat_lines(
    app: &App,
    width: usize,
    view_height: usize,
    placement: ChatPlacement,
) -> Vec<Line<'static>> {
    build_chat_projection(app, width, view_height, placement)
        .lines
        .into_iter()
        .map(|line| line.line)
        .collect()
}

fn build_chat_projection(
    app: &App,
    width: usize,
    view_height: usize,
    placement: ChatPlacement,
) -> ChatProjection {
    let mut out: Vec<ChatProjectedLine> = Vec::new();
    let mut blocks = Vec::new();
    let agent = app.agent();

    if let Some(warning) = agent.selection_warning() {
        append_unselectable(
            &mut out,
            wrap_line_to_width(Line::styled(warning.to_owned(), fg(theme::WARNING)), width),
        );
        append_blank(&mut out);
    }

    if agent.messages().is_empty() {
        let greeting = match placement {
            ChatPlacement::Tab => "Welcome to AI Chat! Type a message below and press Enter.",
            ChatPlacement::Embedded => "Welcome to AI Chat!",
        };
        append_unselectable(
            &mut out,
            wrap_line_to_width(Line::styled(greeting, muted()), width),
        );
        append_blank(&mut out);
        if !crate::llm::is_configured() {
            let note =
                "Note: Run 'teshi auth login' or set TESHI_LLM_API_KEY to enable AI responses.";
            append_unselectable(
                &mut out,
                wrap_line_to_width(Line::styled(note, fg(theme::WARNING)), width),
            );
        }
    }

    let messages = agent.messages();
    let mut message_index = 0;
    while let Some(msg) = messages.get(message_index) {
        if msg.source.as_deref() == Some("acp_activity") {
            let group_end = messages[message_index..]
                .iter()
                .take_while(|message| message.source.as_deref() == Some("acp_activity"))
                .count()
                + message_index;
            let group: Vec<&AiChatMessage> = messages[message_index..group_end].iter().collect();
            let active = matches!(agent.status, AiStatus::Waiting | AiStatus::AwaitingApproval)
                && !messages[group_end..]
                    .iter()
                    .any(|message| matches!(message.role, AiRole::User));
            let mut chunk = Vec::new();
            push_activity_group(
                &mut chunk,
                &group,
                width,
                agent.activity_details_expanded,
                active,
            );
            let source = group
                .iter()
                .map(|message| plain_markdown_text(&message.content))
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            append_selectable_chunk(&mut out, &mut blocks, chunk, message_index, source);
            append_blank(&mut out);
            message_index = group_end;
            continue;
        }
        let mut chunk = Vec::new();
        match msg.role {
            AiRole::Tool => push_tool_result(&mut chunk, msg, width),
            AiRole::User | AiRole::Assistant => push_message(&mut chunk, app, msg, width),
        }
        append_selectable_chunk(
            &mut out,
            &mut blocks,
            chunk,
            message_index,
            block_text_for_message(msg),
        );
        append_blank(&mut out);
        message_index += 1;
    }

    let mut pending = Vec::new();
    push_pending_diff(&mut pending, app, width, view_height);
    append_unselectable(&mut out, pending);

    if !agent.backend.partial_reasoning().is_empty() {
        let mut reasoning = Vec::new();
        push_thinking(
            &mut reasoning,
            agent.backend.partial_reasoning(),
            width,
            agent.activity_details_expanded,
        );
        append_unselectable(&mut out, reasoning);
    }

    if !agent.partial_response().is_empty() {
        let mut body = vec![role_header(AiRole::Assistant, Some("agent_progress"))];
        body.extend(render_markdown_width(
            agent.partial_response(),
            rail_body_width(width),
        ));
        let cursor = Span::styled(
            STREAM_CURSOR,
            fg(theme::ACCENT).add_modifier(Modifier::SLOW_BLINK),
        );
        match body.last_mut() {
            Some(last) => last.spans.push(cursor),
            None => body.push(Line::from(cursor)),
        }
        let mut chunk = Vec::new();
        push_railed(&mut chunk, body, theme::MUTED, width);
        append_selectable_chunk(
            &mut out,
            &mut blocks,
            chunk,
            usize::MAX,
            plain_markdown_text(agent.partial_response()),
        );
        append_blank(&mut out);
    }

    let mut permission = Vec::new();
    push_acp_permission_prompt(&mut permission, app, width);
    append_unselectable(&mut out, permission);
    ChatProjection { lines: out, blocks }
}

fn active_chat_placement(app: &App) -> Option<ChatPlacement> {
    match app.active_tab {
        crate::app::MainTab::Ai => Some(ChatPlacement::Tab),
        crate::app::MainTab::MindMap if app.mindmap_ai_panel_visible => {
            Some(ChatPlacement::Embedded)
        }
        _ => None,
    }
}

/// Map a terminal cell in the active chat history to a stable source anchor.
pub(crate) fn chat_anchor_at(app: &App, col: u16, row: u16) -> Option<ChatAnchor> {
    let placement = active_chat_placement(app)?;
    let rect = app.chat_history_rect?;
    if !rect.contains(ratatui::layout::Position::new(col, row)) {
        return None;
    }
    let projection =
        build_chat_projection(app, rect.width as usize, rect.height as usize, placement);
    let total = projection.lines.len();
    let height = rect.height as usize;
    let max_start = total.saturating_sub(height);
    let start = max_start.saturating_sub(app.agent().scroll_offset.min(max_start));
    projection.anchor_at(
        start + row.saturating_sub(rect.y) as usize,
        col.saturating_sub(rect.x) as usize,
    )
}

/// Reconstruct the selected rendered chat text for the active agent.
pub(crate) fn selected_chat_text(app: &App) -> Option<String> {
    let selection = app.chat_selection?;
    if selection.agent_id != app.selected_agent {
        return None;
    }
    let placement = active_chat_placement(app)?;
    let rect = app.chat_history_rect?;
    let projection =
        build_chat_projection(app, rect.width as usize, rect.height as usize, placement);
    projection.selected_text(selection)
}

fn rail_body_width(width: usize) -> u16 {
    width.saturating_sub(RAIL.width()).min(u16::MAX as usize) as u16
}

fn role_header(role: AiRole, source: Option<&str>) -> Line<'static> {
    let (label, color) = match role {
        AiRole::User => ("You", theme::ACCENT),
        _ if source == Some("agent_progress") => ("Teshi", theme::MUTED),
        _ => ("Teshi", theme::SUCCESS),
    };
    let mut spans = Vec::with_capacity(2);
    if let Some(source) = source.filter(|source| *source != "agent_progress") {
        spans.push(Span::styled(format!("[{source}] "), muted()));
    }
    spans.push(Span::styled(label, fg(color).add_modifier(Modifier::BOLD)));
    Line::from(spans)
}

/// Wraps `lines` to the space right of the rail and prefixes every row with a
/// rail in `color`, so wrapped continuation rows stay visually grouped.
fn push_railed(
    out: &mut Vec<Line<'static>>,
    lines: Vec<Line<'static>>,
    color: ratatui::style::Color,
    width: usize,
) {
    let body_width = rail_body_width(width) as usize;
    let rail = Span::styled(RAIL, fg(color));
    for line in lines {
        for row in wrap_line_to_width(line, body_width) {
            let mut spans = Vec::with_capacity(row.spans.len() + 1);
            spans.push(rail.clone());
            spans.extend(row.spans);
            out.push(Line::from(spans));
        }
    }
}

fn push_message(out: &mut Vec<Line<'static>>, app: &App, msg: &AiChatMessage, width: usize) {
    if msg.source.as_deref() == Some("acp_reasoning") {
        push_thinking(
            out,
            msg.reasoning_content.as_deref().unwrap_or_default(),
            width,
            app.agent().activity_details_expanded,
        );
        return;
    }
    if let Some(reasoning) = &msg.reasoning_content
        && !reasoning.is_empty()
    {
        push_thinking(out, reasoning, width, app.agent().activity_details_expanded);
    }
    let progress = msg.source.as_deref() == Some("agent_progress")
        || msg
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty());
    let mut body = vec![role_header(
        msg.role,
        if progress {
            Some("agent_progress")
        } else {
            msg.source.as_deref()
        },
    )];
    if !msg.content.is_empty() {
        body.extend(render_markdown_width(&msg.content, rail_body_width(width)));
    }
    if let Some(tool_calls) = &msg.tool_calls {
        body.extend(tool_calls.iter().map(|tc| tool_call_line(app, tc)));
    }
    let color = match msg.role {
        AiRole::User => theme::ACCENT,
        _ if progress => theme::MUTED,
        _ => theme::SUCCESS,
    };
    push_railed(out, body, color, width);
}

/// Fits compact activity labels in terminal cells, including wide Unicode.
fn activity_summary(text: &str, width: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.width() <= width {
        return text;
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut cells = 0;
    for ch in text.chars() {
        let size = ch.width().unwrap_or(0);
        if cells + size > width.saturating_sub(1) {
            break;
        }
        result.push(ch);
        cells += size;
    }
    result.push('…');
    result
}

fn push_thinking(out: &mut Vec<Line<'static>>, text: &str, width: usize, expanded: bool) {
    let mut body = vec![Line::styled(
        if width < 42 && expanded {
            "▾ Thinking"
        } else if width < 42 {
            "▸ Thinking"
        } else if expanded {
            "▾ Thinking · /details to collapse"
        } else {
            "▸ Thinking · /details to expand"
        },
        muted(),
    )];
    if expanded {
        body.extend(
            text.lines()
                .map(|line| Line::styled(line.to_owned(), muted())),
        );
    } else if !text.is_empty() {
        body.push(Line::styled(
            activity_summary(text, rail_body_width(width) as usize),
            muted(),
        ));
    }
    push_railed(out, body, theme::MUTED, width);
}

fn push_activity_group(
    out: &mut Vec<Line<'static>>,
    group: &[&AiChatMessage],
    width: usize,
    expanded: bool,
    active: bool,
) {
    let done = group
        .iter()
        .filter(|m| matches!(m.reasoning_content.as_deref(), Some("completed" | "failed")))
        .count();
    let failed = group
        .iter()
        .any(|m| m.reasoning_content.as_deref() == Some("failed"));
    let color = if failed {
        theme::ERROR
    } else if done == group.len() || !active {
        theme::MUTED
    } else {
        theme::WARNING
    };
    let mut body = vec![Line::styled(
        format!(
            "{} Tools ({done}/{}){}",
            if expanded { "▾" } else { "▸" },
            group.len(),
            if width < 42 { "" } else { " · /details" }
        ),
        fg(color),
    )];
    for msg in group {
        let (icon, status, tint) = match msg.reasoning_content.as_deref() {
            Some("completed") => ("✓", "Done", theme::SUCCESS),
            Some("failed") => ("✗", "Failed", theme::ERROR),
            Some("in_progress") if active => (tool_spinner_frame(), "Running", theme::WARNING),
            _ if !active => ("○", "Unconfirmed", theme::MUTED),
            _ => ("○", "Pending", theme::MUTED),
        };
        let prefix = format!("{icon} {status}  ");
        let label = activity_summary(
            &msg.content,
            (rail_body_width(width) as usize).saturating_sub(prefix.width()),
        );
        let show_full_title = expanded && (msg.content.contains('\n') || label != msg.content);
        body.push(Line::from(vec![
            Span::styled(prefix, fg(tint)),
            Span::styled(label, muted()),
        ]));
        if show_full_title {
            body.extend(
                msg.content
                    .lines()
                    .map(|line| Line::styled(format!("    {line}"), muted())),
            );
        }
    }
    push_railed(out, body, color, width);
}

/// One status row per requested tool call, derived from the follow-up tool
/// messages and the pending-approval queue.
fn tool_call_line(app: &App, tc: &ToolCall) -> Line<'static> {
    let agent = app.agent();
    let result = agent
        .messages()
        .iter()
        .find(|m| matches!(m.role, AiRole::Tool) && m.tool_call_id.as_deref() == Some(&tc.id));
    let is_pending = agent.status == AiStatus::AwaitingApproval
        && app
            .pending_agent_changes
            .iter()
            .any(|c| c.tool_call_id == tc.id);
    let (icon, color) = match result {
        Some(m) if m.content.starts_with("Error:") => ("✗", theme::ERROR),
        Some(_) => ("✓", theme::SUCCESS),
        None if is_pending => ("◆", theme::WARNING),
        None => (tool_spinner_frame(), theme::WARNING),
    };
    let mut spans = vec![
        Span::styled(icon, fg(color).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {}", tc.name), muted()),
    ];
    if let Some(ms) = tc.execution_duration_ms {
        let duration = if ms >= 1000 {
            format!("{:.1}s", ms as f64 / 1000.0)
        } else {
            format!("{ms}ms")
        };
        spans.push(Span::styled(format!("  {duration}"), muted()));
    }
    Line::from(spans)
}

fn push_tool_result(out: &mut Vec<Line<'static>>, msg: &AiChatMessage, width: usize) {
    let is_error = msg.content.starts_with("Error:");
    let (icon, color) = if is_error {
        ("✗", theme::ERROR)
    } else {
        ("✓", theme::MUTED)
    };
    let tool_id = msg.tool_call_id.as_deref().unwrap_or("?");
    out.extend(wrap_line_to_width(
        Line::from(vec![
            Span::raw("  "),
            Span::styled(icon, fg(color).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" Tool ({tool_id})"), muted()),
        ]),
        width,
    ));

    // Tool output is supporting detail: default its text to the muted color
    // while keeping any explicit Markdown colors (code, headings).
    let indent = TOOL_RESULT_INDENT.width();
    let body_width = width.saturating_sub(indent);
    let body = render_markdown_width(&msg.content, body_width.min(u16::MAX as usize) as u16);
    for line in body {
        for row in wrap_line_to_width(line, body_width) {
            let mut spans = vec![Span::raw(TOOL_RESULT_INDENT)];
            spans.extend(row.spans.into_iter().map(|s| {
                let style = muted().patch(s.style);
                s.style(style)
            }));
            out.push(Line::from(spans));
        }
    }
}

fn push_pending_diff(out: &mut Vec<Line<'static>>, app: &App, width: usize, view_height: usize) {
    if app.agent().status != AiStatus::AwaitingApproval || app.pending_change_diffs.is_empty() {
        return;
    }
    let total = app.pending_change_diffs.len();
    let title = if total > 1 {
        format!("◆ Pending Change (1 of {total})")
    } else {
        "◆ Pending Change".to_string()
    };
    out.push(Line::default());
    out.extend(wrap_line_to_width(
        Line::from(vec![
            Span::raw("  "),
            Span::styled(title, fg(theme::WARNING).add_modifier(Modifier::BOLD)),
        ]),
        width,
    ));
    if let Some(diff) = app.pending_change_diffs.first() {
        let max_lines = view_height.saturating_sub(out.len()).clamp(3, 20);
        for dl in diff.iter().take(max_lines) {
            let (prefix, color) = match dl.kind {
                ChangeKind::Added => ("+", theme::SUCCESS),
                ChangeKind::Modified => ("~", theme::WARNING),
                ChangeKind::Deleted => ("-", theme::ERROR),
                ChangeKind::Unchanged => (" ", theme::MUTED),
            };
            out.extend(wrap_line_to_width(
                Line::styled(format!("    {prefix} {}", dl.text), fg(color)),
                width,
            ));
        }
    }
    out.push(Line::default());
}

/// Appends the pending ACP permission prompt, if the active agent has one.
/// Mirrors Zed's agent permission panel: a highlighted option list with
/// Enter to allow and Esc to reject.
fn push_acp_permission_prompt(out: &mut Vec<Line<'static>>, app: &App, width: usize) {
    let Some(prompt) = app.agent().pending_acp_permission.as_ref() else {
        return;
    };
    out.push(Line::default());
    out.extend(wrap_line_to_width(
        Line::styled(
            "  ◆ ACP agent requests permission",
            fg(theme::WARNING).add_modifier(Modifier::BOLD),
        ),
        width,
    ));
    for (index, option) in prompt.options.iter().enumerate() {
        let line = if index == prompt.selected {
            Line::from(format!("    ▶ {}", option.display_label())).style(selected_row())
        } else {
            Line::styled(format!("      {}", option.display_label()), fg(theme::FG))
        };
        out.extend(wrap_line_to_width(line, width));
    }
    out.extend(wrap_line_to_width(
        Line::styled(
            "    Enter: allow · Esc: reject · j/k: move",
            muted().add_modifier(Modifier::ITALIC),
        ),
        width,
    ));
    out.push(Line::default());
}

/// Renders the one-row status line: activity on the left, model tag on the right.
fn render_chat_status(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if app.agent().task_interrupted() {
        let label = "[Continue task]";
        let button_width = (label.len() as u16).min(area.width);
        let button = Rect::new(area.right() - button_width, area.y, button_width, 1);
        frame.render_widget(
            Paragraph::new(Line::styled(
                label,
                fg(theme::WARNING).add_modifier(Modifier::BOLD),
            )),
            button,
        );
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect: button,
            action: crate::keymap::Action::AiContinueTask,
            modal: false,
        });
        let reason = app
            .agent()
            .last_finish_reason
            .as_deref()
            .unwrap_or("unknown");
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!("Task interrupted ({reason}) · /retry"),
                fg(theme::WARNING),
            )),
            Rect::new(
                area.x,
                area.y,
                area.width.saturating_sub(button_width + 1),
                1,
            ),
        );
        return;
    }
    let agent = app.agent();
    let thinking = agent.status == AiStatus::Waiting
        && (agent.partial_response().is_empty() || agent.backend.acp_controls().is_some());
    let status_text: String = match agent.status {
        AiStatus::Waiting if thinking => agent
            .tool_status
            .clone()
            .unwrap_or_else(|| "Teshi is thinking...".to_string()),
        AiStatus::AwaitingApproval => {
            if app.has_acp_permission_prompt() {
                "◆ ACP permission requested — Enter to allow, Esc to reject".into()
            } else if matches!(app.generation_stage, GenerationStage::Confirming) {
                format!(
                    "◆ Changes pending — Press Y to accept, N to reject · {}",
                    app.generation_stage.label()
                )
            } else {
                "◆ Waiting for approval — Y/N".into()
            }
        }
        AiStatus::Error => {
            if app.status.starts_with("AI error:") {
                app.status.clone()
            } else {
                "AI error — check TESHI_LLM_API_KEY and your network connection.".to_string()
            }
        }
        _ => String::new(),
    };

    let mut left: Vec<Span<'static>> = Vec::new();
    match agent.status {
        AiStatus::Waiting if thinking => {
            left.push(Span::styled(
                thinking_pulse_frame(),
                fg(theme::ACCENT).add_modifier(Modifier::BOLD),
            ));
            left.push(Span::styled(format!(" {status_text}"), fg(theme::FG)));
        }
        AiStatus::AwaitingApproval => {
            left.push(Span::styled(status_text, fg(theme::WARNING)));
        }
        AiStatus::Error => {
            left.push(Span::styled(status_text, fg(theme::ERROR)));
            left.push(Span::styled("  Enter retry · Esc clear", muted()));
        }
        _ => {}
    }
    // Pipeline stage is redundant while approval text already names it.
    if !matches!(
        app.generation_stage,
        GenerationStage::Idle | GenerationStage::Complete
    ) && agent.status != AiStatus::AwaitingApproval
    {
        let sep = if left.is_empty() { "" } else { " · " };
        left.push(Span::styled(
            format!("{sep}{}", app.generation_stage.label()),
            muted(),
        ));
    }

    let model_label = if app.agent().backend.acp_controls().is_some() {
        ""
    } else {
        app.active_model_label.as_deref().unwrap_or("")
    };
    let model_tag = (!model_label.is_empty()).then(|| format!(" {model_label} "));
    let tag_width = model_tag.as_deref().map_or(0, |t| t.width() as u16);
    let [status_left, status_right] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(tag_width)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(left)), status_left);
    if let Some(tag) = model_tag {
        frame.render_widget(
            Paragraph::new(Line::styled(
                tag,
                fg(theme::ACCENT).add_modifier(Modifier::BOLD),
            )),
            status_right,
        );
    }
}

/// Returns the slash-command filter text when suggestions should be shown.
fn slash_filter(app: &App, placement: ChatPlacement) -> Option<String> {
    (placement == ChatPlacement::Tab && app.slash_suggestion_active).then(|| {
        app.agent()
            .input
            .strip_prefix('/')
            .unwrap_or("")
            .to_lowercase()
    })
}

fn filtered_slash_commands(filter: &str) -> Vec<&'static (&'static str, &'static str)> {
    SLASH_COMMANDS
        .iter()
        .filter(|(name, _)| filter.is_empty() || name.starts_with(filter))
        .collect()
}

fn render_slash_suggestions(frame: &mut Frame<'_>, app: &mut App, area: Rect, filter: &str) {
    let filtered = filtered_slash_commands(filter);
    super::pointer_controls::render_slash_suggestions(frame, app, area, &filtered);
}

fn render_chat_input(frame: &mut Frame<'_>, app: &mut App, area: Rect, active: bool) {
    app.clickable_regions
        .push(crate::app::ClickableRegion::ChatInput { rect: area });
    let border_color = if active { theme::ACCENT } else { theme::BORDER };
    let agent = app.agent();
    let usage = token_usage_label(agent.total_input_tokens, agent.total_output_tokens);
    let context = match agent.last_input_tokens {
        Some(tokens) => format!(
            "ctx {tokens}/{}{}",
            app.active_context_window(),
            if u64::from(tokens) * 10 >= u64::from(app.active_context_window()) * 7 {
                " !"
            } else {
                ""
            }
        ),
        None => "ctx unknown".to_string(),
    };
    let seconds = agent
        .task_started
        .map_or(agent.task_elapsed, |start| start.elapsed())
        .as_secs();
    let runtime = format!("{}m{:02}s", seconds / 60, seconds % 60);
    let label = if area.width >= 75 {
        format!("{runtime} · {context} · {usage}")
    } else {
        format!("{runtime} · {context}")
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(fg(border_color));
    if !label.is_empty() {
        block = block
            .title_top(Line::styled(format!(" {label} "), muted()).alignment(Alignment::Right));
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let (rows, cursor) = input_rows(&agent.input, agent.input_cursor, inner.width);
    let scroll = cursor.0.saturating_sub(inner.height - 1);

    let display: Text<'static> = if agent.input.is_empty() {
        if active {
            Text::from(Line::styled("Type your message...", muted()))
        } else {
            Text::default()
        }
    } else {
        Text::from(
            rows.into_iter()
                .skip(usize::from(scroll))
                .collect::<Vec<_>>(),
        )
    };
    let text_style = match agent.status {
        AiStatus::Waiting => muted(),
        _ => Style::default(),
    };
    frame.render_widget(Paragraph::new(display).style(text_style), inner);

    // Show a visible cursor at the insertion point, accounting for wrapping.
    if active && agent.status != AiStatus::Waiting {
        frame.set_cursor_position((inner.x + cursor.1, inner.y + cursor.0 - scroll));
    }
}

/// Number of visual (wrapped) rows the input text occupies when rendered in a
/// widget of the given `width` (in columns).
fn visual_line_count_for_width(input: &str, width: u16) -> u16 {
    input_rows(input, input.chars().count(), width)
        .0
        .len()
        .min(usize::from(u16::MAX)) as u16
}

/// Use the same cell wrapping for text and cursor, including trailing empty lines.
fn input_rows(input: &str, cursor: usize, width: u16) -> (Vec<Line<'static>>, (u16, u16)) {
    let width = usize::from(width.max(1));
    let mut rows = vec![String::new()];
    let mut column = 0;
    let mut position = (0, 0);
    for (index, ch) in input.chars().enumerate() {
        let cells = if ch == '\t' {
            4 - column % 4
        } else {
            ch.width().unwrap_or(0).min(width)
        };
        if ch != '\n' && column + cells > width {
            rows.push(String::new());
            column = 0;
        }
        if index == cursor {
            position = (
                (rows.len() - 1).min(usize::from(u16::MAX)) as u16,
                column as u16,
            );
        }
        if ch == '\n' {
            rows.push(String::new());
            column = 0;
        } else {
            if let Some(row) = rows.last_mut() {
                if ch == '\t' {
                    row.push_str(&" ".repeat(cells));
                } else if !ch.is_control() {
                    row.push(ch);
                }
            }
            column += cells;
            if column == width {
                rows.push(String::new());
                column = 0;
            }
        }
    }
    if cursor >= input.chars().count() {
        position = (
            (rows.len() - 1).min(usize::from(u16::MAX)) as u16,
            column as u16,
        );
    }
    (rows.into_iter().map(Line::raw).collect(), position)
}

/// Compute the insertion position using the input renderer's cell wrapping.
#[cfg(test)]
fn visual_cursor_pos(input: &str, cursor_char_idx: usize, width: u16) -> (u16, u16) {
    if width == 0 {
        return (0, 0);
    }
    input_rows(input, cursor_char_idx, width).1
}

/// Wraps one styled line into rows no wider than `width` display cells.
///
/// Breaks after the last whitespace when possible and hard-breaks otherwise,
/// measuring with `unicode-width` so double-width CJK characters are never
/// split. The line's own style is folded into each span, and lines whose style
/// sets a background are padded to `width` so block surfaces stay rectangular.
/// A `width` of `0` returns the line unchanged.
pub(super) fn wrap_line_to_width(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![line];
    }
    let line_style = line.style;
    let pad_style = line_style.bg.is_some().then_some(line_style);
    let cells: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|span| {
            let style = line_style.patch(span.style);
            span.content.chars().map(move |c| (c, style))
        })
        .collect();

    let cell_width = |c: char| c.width().unwrap_or(0);
    let mut rows: Vec<Vec<(char, Style)>> = Vec::new();
    let mut row: Vec<(char, Style)> = Vec::new();
    let mut row_width = 0usize;
    // Index just past the last whitespace in `row`: the preferred break point.
    let mut break_at: Option<usize> = None;

    for (c, style) in cells {
        let w = cell_width(c);
        // Overflowing whitespace hangs off the row end instead of starting
        // the next row with a stray space; render clipping hides it.
        if c.is_whitespace() && row_width + w > width {
            row.push((c, style));
            row_width += w;
            break_at = Some(row.len());
            continue;
        }
        while row_width + w > width && !row.is_empty() {
            let split = break_at.take().unwrap_or(row.len());
            let rest = row.split_off(split);
            rows.push(std::mem::replace(&mut row, rest));
            row_width = row.iter().map(|&(c, _)| cell_width(c)).sum();
        }
        row.push((c, style));
        row_width += w;
        if c.is_whitespace() {
            break_at = Some(row.len());
        }
    }
    rows.push(row);

    rows.into_iter()
        .map(|cells| build_row(cells, width, pad_style))
        .collect()
}

/// Merges runs of equally styled cells back into spans.
fn build_row(cells: Vec<(char, Style)>, width: usize, pad_style: Option<Style>) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut current: Option<Style> = None;
    let mut used = 0usize;
    for (c, style) in cells {
        used += c.width().unwrap_or(0);
        if current.is_some_and(|s| s != style) {
            spans.push(Span::styled(
                std::mem::take(&mut buf),
                current.unwrap_or_default(),
            ));
        }
        current = Some(style);
        buf.push(c);
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, current.unwrap_or_default()));
    }
    if let Some(style) = pad_style
        && used < width
    {
        spans.push(Span::styled(" ".repeat(width - used), style));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    #[test]
    fn progress_header_is_muted_and_final_header_is_green() {
        let progress = role_header(AiRole::Assistant, Some("agent_progress"));
        assert_eq!(progress.spans.len(), 1);
        assert_eq!(progress.spans[0].style.fg, Some(theme::MUTED));
        let final_reply = role_header(AiRole::Assistant, None);
        assert_eq!(final_reply.spans[0].style.fg, Some(theme::SUCCESS));
    }

    use super::*;

    fn activity_test_app() -> (App, tempfile::TempDir, tempfile::TempDir) {
        let project = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let feature = project.path().join("sample.feature");
        std::fs::write(&feature, "Feature: Sample\n").unwrap();
        let mut app = App::from_file(
            &feature,
            crate::config::load_config().unwrap(),
            store.path().to_path_buf(),
        )
        .unwrap();
        app.agents[0] = crate::app::AgentThread::new_with_backend(
            0,
            "Native",
            teshi_agent::backend::AgentBackendKind::Native,
        );
        (app, project, store)
    }

    #[test]
    fn input_title_keeps_elapsed_time_and_unknown_usage_without_focus_hint() {
        use ratatui::{Terminal, backend::TestBackend};
        let (mut app, _project, _store) = activity_test_app();
        app.agents[0].task_elapsed = std::time::Duration::from_secs(125);
        app.agents[0].task_started = None;
        app.agents[0].last_input_tokens = None;
        let mut terminal = Terminal::new(TestBackend::new(100, 6)).unwrap();
        for focused in [true, false] {
            terminal
                .draw(|frame| render_chat_input(frame, &mut app, frame.area(), focused))
                .unwrap();
            let screen: String = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(screen.contains("2m05s"), "{screen}");
            assert!(screen.contains("ctx unknown"), "{screen}");
            assert!(!screen.contains("Enter send"), "{screen}");
        }
    }

    #[test]
    fn context_warning_uses_last_input_and_seventy_percent_boundary() {
        use ratatui::{Terminal, backend::TestBackend};
        let (mut app, _project, _store) = activity_test_app();
        let limit = app.active_context_window();
        let threshold = (u64::from(limit) * 7).div_ceil(10) as u32;
        let mut terminal = Terminal::new(TestBackend::new(100, 6)).unwrap();
        for (tokens, warning) in [(threshold - 1, false), (threshold, true)] {
            app.agents[0].last_input_tokens = Some(tokens);
            app.agents[0].total_input_tokens = 0;
            terminal
                .draw(|frame| render_chat_input(frame, &mut app, frame.area(), true))
                .unwrap();
            let screen: String = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            let label = format!("ctx {tokens}/{limit}");
            assert!(screen.contains(&label), "{screen}");
            assert_eq!(screen.contains(&format!("{label} !")), warning, "{screen}");
        }
    }

    #[test]
    fn chat_selection_uses_plain_message_projection_across_wrap_widths() {
        let (mut app, _project, _store) = activity_test_app();
        app.agents[0].messages_mut().push(AiChatMessage {
            role: AiRole::Assistant,
            content: "Answer **with** CJK 内容 and a long tail".into(),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            source: None,
        });

        let narrow = build_chat_projection(&app, 24, 20, ChatPlacement::Tab);
        let selectable_lines: Vec<_> = narrow
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.source.as_ref().is_some_and(|source| source.block == 0))
            .collect();
        let (first_index, first_line) = selectable_lines
            .first()
            .expect("assistant body should be selectable");
        let (last_index, last_line) = selectable_lines
            .last()
            .expect("wrapped assistant body should remain selectable");
        let first_source = first_line.source.as_ref().unwrap();
        let last_source = last_line.source.as_ref().unwrap();
        let start = narrow
            .anchor_at(*first_index, first_source.display_start)
            .unwrap();
        let end = narrow
            .anchor_at(*last_index, last_source.display_end)
            .unwrap();
        let selection = ChatSelection {
            agent_id: 0,
            anchor: start,
            focus: end,
        };
        assert_eq!(
            narrow.selected_text(selection).as_deref(),
            Some("Answer with CJK 内容 and a long tail")
        );
        assert!(
            narrow
                .highlighted_line(*first_index, Some(selection))
                .spans
                .iter()
                .any(|span| span.style.bg == Some(theme::CURSOR_BG))
        );

        let wide = build_chat_projection(&app, 80, 20, ChatPlacement::Tab);
        assert!(wide.selection_valid(selection));
        assert_eq!(
            wide.selected_text(selection).as_deref(),
            Some("Answer with CJK 内容 and a long tail")
        );
    }

    #[test]
    fn chat_selection_reconstructs_source_newlines_and_reverse_drags() {
        let (mut app, _project, _store) = activity_test_app();
        app.agents[0].messages_mut().extend([
            AiChatMessage {
                role: AiRole::Assistant,
                content: "first\nsecond".into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: None,
            },
            AiChatMessage {
                role: AiRole::Assistant,
                content: "third **answer**".into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: None,
            },
        ]);

        let projection = build_chat_projection(&app, 40, 30, ChatPlacement::Tab);
        let (first_index, first_line) = projection
            .lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.source.as_ref().is_some_and(|source| source.block == 0))
            .expect("first assistant body should be selectable");
        let (last_index, last_line) = projection
            .lines
            .iter()
            .enumerate()
            .rfind(|(_, line)| line.source.as_ref().is_some_and(|source| source.block == 1))
            .expect("last assistant body should be selectable");
        let first_source = first_line.source.as_ref().unwrap();
        let last_source = last_line.source.as_ref().unwrap();
        let start = projection
            .anchor_at(first_index, first_source.display_start)
            .unwrap();
        let end = projection
            .anchor_at(last_index, last_source.display_end)
            .unwrap();
        let selection = ChatSelection {
            agent_id: 0,
            anchor: start,
            focus: end,
        };
        assert_eq!(
            projection.selected_text(selection).as_deref(),
            Some("first\nsecond\n\nthird answer")
        );
        assert_eq!(
            projection
                .selected_text(ChatSelection {
                    agent_id: 0,
                    anchor: end,
                    focus: start,
                })
                .as_deref(),
            Some("first\nsecond\n\nthird answer")
        );
    }

    #[test]
    fn chat_selection_mouse_anchor_matches_rendered_cell() {
        let (mut app, _project, _store) = activity_test_app();
        app.active_tab = crate::app::MainTab::Ai;
        app.agents[0].messages_mut().extend([
            AiChatMessage {
                role: AiRole::User,
                content: "first response".into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: None,
            },
            AiChatMessage {
                role: AiRole::Assistant,
                content: "WRAPPED_REPLY_01".into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: None,
            },
        ]);

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();
        let marker = "WRAPPED_REPLY_01";
        let (x, y) = (0..100)
            .flat_map(|x| (0..30).map(move |y| (x, y)))
            .find(|&(x, y)| {
                let row: String = (0..100)
                    .map(|column| terminal.backend().buffer()[(column, y)].symbol())
                    .collect();
                row.get(x as usize..)
                    .is_some_and(|suffix| suffix.starts_with(marker))
            })
            .expect("assistant marker must be rendered");
        assert!(
            app.chat_history_rect
                .expect("chat history rect")
                .contains(ratatui::layout::Position::new(x, y))
        );
        assert!(chat_anchor_at(&app, x, y).is_some());
        app.model_panel_active = true;
        app.handle_mouse_event(
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            x,
            y,
            crossterm::event::KeyModifiers::NONE,
        )
        .unwrap();
        assert!(app.chat_selection.is_none());
        app.model_panel_active = false;
        app.handle_mouse_event(
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            x,
            y,
            crossterm::event::KeyModifiers::NONE,
        )
        .unwrap();
        assert!(app.chat_selection.is_some());
    }

    fn assert_reply_color(app: &App, text: &str, expected: ratatui::style::Color) {
        let rows = build_chat_lines(app, 100, 60, ChatPlacement::Tab);
        let texts = row_texts(&rows);
        let indices: Vec<_> = texts
            .iter()
            .enumerate()
            .filter(|(_, row)| row.contains(text))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(indices.len(), 1, "reply must occur once: {texts:?}");
        let index = indices[0];
        assert_eq!(rows[index].spans[0].style.fg, Some(expected), "reply rail");
        let header = (0..index)
            .rev()
            .find(|i| texts[*i].contains("Teshi"))
            .unwrap();
        assert_eq!(
            rows[header].spans[0].style.fg,
            Some(expected),
            "header rail"
        );
        assert_eq!(
            rows[header].spans.last().unwrap().style.fg,
            Some(expected),
            "header text"
        );
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 60)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(Paragraph::new(Text::from(rows)), frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(0, index as u16)].fg,
            expected,
            "rendered reply rail"
        );
        assert_eq!(
            buffer[(0, header as u16)].fg,
            expected,
            "rendered header rail"
        );
    }

    #[test]
    fn native_stream_completion_failure_and_cancellation_match_feature_styles() {
        use teshi_engine::llm::LlmEvent;
        for ending in ["normal", "failure", "cancellation"] {
            let (mut app, _project, _store) = activity_test_app();
            let (tx, rx) = std::sync::mpsc::channel();
            app.agents[0]
                .backend
                .attach_model_event_receiver_for_tests(rx);
            app.agents[0].backend.start().unwrap();
            app.agents[0].status = AiStatus::Waiting;
            tx.send(LlmEvent::Chunk {
                content: "Partial result".into(),
            })
            .unwrap();
            app.poll_llm_events();
            assert_reply_color(&app, "Partial result", theme::MUTED);
            match ending {
                "normal" => tx
                    .send(LlmEvent::Done {
                        full_text: "Partial result".into(),
                        reasoning_content: None,
                        input_tokens: None,
                        output_tokens: None,
                        model: "controlled".into(),
                        finish_reason: Some("stop".into()),
                    })
                    .unwrap(),
                "failure" => tx
                    .send(LlmEvent::Error {
                        message: "controlled failure".into(),
                    })
                    .unwrap(),
                _ => app.handle_action(crate::keymap::Action::AiCancel).unwrap(),
            }
            app.poll_llm_events();
            assert_reply_color(
                &app,
                "Partial result",
                if ending == "normal" {
                    theme::SUCCESS
                } else {
                    theme::MUTED
                },
            );
            if ending == "cancellation" {
                assert_reply_color(&app, "[cancelled", theme::MUTED);
            }
        }
    }

    fn poll_activity_until(app: &mut App, condition: impl Fn(&App) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            app.poll_llm_events();
            if condition(app) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "activity timed out: {}",
                app.status
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn acp_stream_tools_completion_and_interruptions_match_feature_styles() {
        use teshi_agent::backend::{AgentBackendKind, AgentBackendStatus};
        use teshi_agent_runtime::backend::{AcpBackendConfig, AgentBackendRuntime};
        for mode in [
            "visual_success",
            "visual_failed",
            "visual_cancelled",
            "visual_no_final",
        ] {
            let (mut app, project, _store) = activity_test_app();
            let gate = project.path().join("gate");
            let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../teshi-agent-runtime/tests/fixtures/fake_acp_agent.py");
            let config = AcpBackendConfig::new(teshi_acp::AcpAgentCommand {
                program: if cfg!(windows) { "python" } else { "python3" }.into(),
                args: vec![script.to_string_lossy().into_owned(), mode.into()],
                cwd: project.path().into(),
                env: [(
                    "TESHI_ACTIVITY_GATE".into(),
                    gate.to_string_lossy().into_owned(),
                )]
                .into(),
            });
            app.agents[0].backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 2);
            app.agents[0].backend.configure_acp(config).unwrap();
            app.agents[0].backend.start().unwrap();
            app.agents[0].status = AiStatus::Waiting;
            app.agents[0]
                .backend
                .submit_turn(
                    None,
                    vec![teshi_core::llm::ChatMessage::text("user", "Inspect")],
                    None,
                )
                .unwrap();
            poll_activity_until(&mut app, |app| !app.agent().partial_response().is_empty());
            assert_reply_color(&app, "I will inspect the project", theme::MUTED);
            std::fs::write(gate.with_extension("tools"), "continue").unwrap();
            if mode == "visual_success" {
                poll_activity_until(&mut app, |app| {
                    app.agent().partial_response() == "Inspection finished"
                });
                assert_reply_color(&app, "I will inspect the project", theme::MUTED);
                assert_reply_color(&app, "Inspection finished", theme::MUTED);
                std::fs::write(gate.with_extension("finish"), "continue").unwrap();
            }
            poll_activity_until(&mut app, |app| {
                app.agent().backend.status() != AgentBackendStatus::Running
            });
            assert_reply_color(&app, "I will inspect the project", theme::MUTED);
            if mode == "visual_success" || mode == "visual_no_final" {
                let rows = row_texts(&build_chat_lines(&app, 100, 60, ChatPlacement::Tab));
                let progress = rows
                    .iter()
                    .position(|row| row.contains("I will inspect the project"))
                    .unwrap();
                let tool = rows
                    .iter()
                    .position(|row| row.contains("Done  Read project"))
                    .unwrap();
                assert!(progress < tool, "{rows:?}");
                assert_eq!(
                    app.agent()
                        .messages()
                        .iter()
                        .filter(|m| m.tool_call_id.as_deref() == Some("read-project"))
                        .count(),
                    1
                );
                if mode == "visual_success" {
                    assert_reply_color(&app, "Inspection finished", theme::SUCCESS);
                    assert!(
                        tool < rows
                            .iter()
                            .position(|row| row.contains("Inspection finished"))
                            .unwrap()
                    );
                } else {
                    assert!(!rows.iter().any(|row| row.contains("Inspection finished")));
                }
            }
        }
    }

    fn row_texts(rows: &[Line<'_>]) -> Vec<String> {
        rows.iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn test_wrap_line_to_width_table() {
        let cases: &[(&str, usize, &[&str])] = &[
            ("hello world", 5, &["hello ", "world"]),
            ("abcdef", 3, &["abc", "def"]),
            ("short", 10, &["short"]),
            ("你好世界", 5, &["你好", "世界"]),
            ("ab 你好世界", 4, &["ab ", "你好", "世界"]),
            ("", 4, &[""]),
            ("unchanged text", 0, &["unchanged text"]),
        ];
        for (input, width, expected) in cases {
            let rows = wrap_line_to_width(Line::raw(*input), *width);
            assert_eq!(
                row_texts(&rows),
                expected.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                "input {input:?} at width {width}"
            );
            if *width > 0 {
                for row in &rows {
                    assert!(row.width() <= *width + 1, "row too wide: {row:?}");
                }
            }
        }
    }

    #[test]
    fn test_wrap_line_to_width_preserves_span_styles() {
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let line = Line::from(vec![Span::styled("abcd", bold), Span::raw("ef")]);
        let rows = wrap_line_to_width(line, 3);
        assert_eq!(row_texts(&rows), vec!["abc", "def"]);
        assert_eq!(rows[0].spans[0].style, bold);
        assert_eq!(rows[1].spans[0].content, "d");
        assert_eq!(rows[1].spans[0].style, bold);
        assert_eq!(rows[1].spans[1].style, Style::default());
    }

    #[test]
    fn test_wrap_line_to_width_pads_background_lines() {
        let bg = Style::default().bg(theme::BLOCK_BG);
        let rows = wrap_line_to_width(Line::from("abcdefg").style(bg), 5);
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row.width(), 5, "background row not padded: {row:?}");
            assert_eq!(row.style, Style::default(), "line style folded into spans");
            assert!(row.spans.iter().all(|s| s.style.bg == bg.bg));
        }
    }

    #[test]
    fn test_push_railed_prefixes_every_wrapped_row() {
        let mut out = Vec::new();
        let long = Line::raw("one two three four five six seven");
        push_railed(&mut out, vec![long], theme::ACCENT, 12);
        assert!(out.len() > 1, "expected wrapping, got {out:?}");
        for row in &out {
            assert_eq!(row.spans[0].content, RAIL);
            assert_eq!(row.spans[0].style.fg, Some(theme::ACCENT));
            assert!(row.width() <= 12 + 1, "row too wide: {row:?}");
        }
    }

    #[test]
    fn test_chat_panel_long_message_keeps_rail_on_wrapped_rows() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let project = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let feature = project.path().join("sample.feature");
        std::fs::write(&feature, "Feature: Sample\n").unwrap();
        let mut app = App::from_file(
            &feature,
            crate::config::load_config().unwrap(),
            store.path().to_path_buf(),
        )
        .unwrap();
        app.agents[0].messages_mut().push(AiChatMessage {
            role: AiRole::User,
            content: "word ".repeat(30),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            source: None,
        });

        let mut terminal = Terminal::new(TestBackend::new(40, 24)).unwrap();
        terminal
            .draw(|f| render_chat_panel(f, &mut app, f.area(), true, ChatPlacement::Tab))
            .unwrap();
        let buffer = terminal.backend().buffer();

        // Panel border (x=0) + 1 column of padding puts the rail at x=2.
        let rail_x = 2;
        let message_rows: Vec<u16> = (0..buffer.area.height)
            .filter(|&y| {
                let row: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                row.contains("word") || row.contains("You")
            })
            .collect();
        assert!(
            message_rows.len() > 2,
            "message should wrap across several rows, got {message_rows:?}"
        );
        for y in message_rows {
            assert_eq!(buffer[(rail_x, y)].symbol(), "▌", "row {y} missing rail");
            assert_eq!(buffer[(rail_x, y)].fg, theme::ACCENT);
        }
    }

    #[test]
    fn reasoning_and_acp_tool_status_are_visible_in_chat_history() {
        let project = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let feature = project.path().join("sample.feature");
        std::fs::write(&feature, "Feature: Sample\n").unwrap();
        let mut app = App::from_file(
            &feature,
            crate::config::load_config().unwrap(),
            store.path().to_path_buf(),
        )
        .unwrap();
        let thought = AiChatMessage {
            role: AiRole::Assistant,
            content: "Answer".into(),
            reasoning_content: Some("Inspecting the project".into()),
            source: None,
            tool_calls: None,
            tool_call_id: None,
        };
        let tool = AiChatMessage {
            role: AiRole::Assistant,
            content: "Read project".into(),
            reasoning_content: Some("completed".into()),
            source: Some("acp_activity".into()),
            tool_calls: None,
            tool_call_id: Some("read-1".into()),
        };
        app.agents[0].messages_mut().extend([thought, tool]);
        let (tx, rx) = std::sync::mpsc::channel();
        app.agents[0]
            .backend
            .attach_model_event_receiver_for_tests(rx);
        app.agents[0].backend.start().unwrap();
        tx.send(teshi_engine::llm::LlmEvent::ReasoningChunk {
            content: "Live reasoning before completion".into(),
        })
        .unwrap();
        app.poll_llm_events();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(72, 24)).unwrap();
        terminal
            .draw(|f| render_chat_panel(f, &mut app, f.area(), true, ChatPlacement::Tab))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let screen = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        for expected in [
            "Thinking",
            "Inspecting the project",
            "Answer",
            "Tools (1/1)",
            "Done  Read project",
            "Live reasoning before completion",
        ] {
            assert!(screen.contains(expected), "Missing {expected}: {screen}");
        }
    }

    #[test]
    fn compact_activity_keeps_status_and_expands_full_unicode_title() {
        let title = "Read 项目目录 with a very long command and all its arguments";
        let tool = AiChatMessage {
            role: AiRole::Assistant,
            content: title.into(),
            reasoning_content: Some("failed".into()),
            source: Some("acp_activity".into()),
            tool_calls: None,
            tool_call_id: Some("read".into()),
        };
        let mut compact = Vec::new();
        push_activity_group(&mut compact, &[&tool], 34, false, false);
        let compact_text = row_texts(&compact).join("\n");
        assert!(compact_text.contains("Failed"));
        assert!(compact_text.contains('…'));
        assert!(!compact_text.contains("all its arguments"));
        assert!(compact.iter().all(|line| line.width() <= 34));
        let mut expanded = Vec::new();
        push_activity_group(&mut expanded, &[&tool], 100, true, false);
        assert!(row_texts(&expanded).join("\n").contains(title));
        let mut thoughts = Vec::new();
        push_thinking(&mut thoughts, "First line\nSecond line", 72, false);
        assert_eq!(thoughts.len(), 2);
        assert!(
            thoughts
                .iter()
                .flat_map(|line| &line.spans)
                .all(|span| span.style.fg == Some(theme::MUTED))
        );
        thoughts.clear();
        push_thinking(&mut thoughts, "First line\nSecond line", 72, true);
        assert_eq!(thoughts.len(), 3);
    }

    #[test]
    fn test_input_rows_preserve_empty_lines_and_wide_character_cursor() {
        let (rows, cursor) = input_rows("hello\n\n", 7, 10);
        assert_eq!(rows.len(), 3);
        assert_eq!(cursor, (2, 0));
        assert_eq!(visual_line_count_for_width("hello\n", 10), 2);
        let (rows, cursor) = input_rows("ab你c", 2, 3);
        assert_eq!(
            rows.iter().map(ToString::to_string).collect::<Vec<_>>(),
            vec!["ab", "你c", ""]
        );
        assert_eq!(cursor, (1, 0));
        assert_eq!(input_rows("a\tb", 3, 8).1, (0, 5));
    }

    #[test]
    fn test_visual_cursor_pos_wraps_and_counts_lines() {
        assert_eq!(visual_cursor_pos("abcdef", 6, 4), (1, 2));
        assert_eq!(visual_cursor_pos("ab\ncd", 4, 10), (1, 1));
        assert_eq!(visual_cursor_pos("anything", 3, 0), (0, 0));
    }
}
