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
/// Guide for thinking, tool details and replies that have not finished; the same two-cell
/// source offset.
const QUIET_RAIL: &str = "│ ";
/// Indent from the guide to a tool result's text, keeping it a step inside the call row.
const TOOL_RESULT_INDENT: &str = "  ";
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
    action: Option<crate::keymap::Action>,
    image: Option<ImageRow>,
}

/// One row of a cell box reserved for an attached image.
#[derive(Clone)]
struct ImageRow {
    path: std::rc::Rc<std::path::PathBuf>,
    /// Row within the box, from 0.
    row: u16,
    /// Box size in cells.
    size: (u16, u16),
}

/// Tallest image box in the transcript, in rows.
const IMAGE_BOX_MAX_ROWS: u16 = 16;

/// Cell box for an image of `pixels`, fitted within `max_cols` x
/// [`IMAGE_BOX_MAX_ROWS`] with its aspect ratio kept.
///
/// The box comes from the file header, not the decoded image, so the layout
/// is final before the pixels arrive. Small images keep their native size:
/// enlarging a screenshot only blurs it.
fn image_box(
    (width, height): (u32, u32),
    (font_w, font_h): (u16, u16),
    max_cols: u16,
) -> (u16, u16) {
    if width == 0 || height == 0 || max_cols == 0 {
        return (0, 0);
    }
    let (font_w, font_h) = (f64::from(font_w.max(1)), f64::from(font_h.max(1)));
    let (width, height) = (f64::from(width), f64::from(height));
    let scale = (f64::from(max_cols) * font_w / width)
        .min(f64::from(IMAGE_BOX_MAX_ROWS) * font_h / height)
        .min(1.0);
    let cells =
        |pixels: f64, font: f64, max: u16| ((pixels * scale / font).ceil() as u16).clamp(1, max);
    (
        cells(width, font_w, max_cols),
        cells(height, font_h, IMAGE_BOX_MAX_ROWS),
    )
}

#[derive(Clone)]
struct ChatBlock {
    key: usize,
    text: std::rc::Rc<String>,
}

#[derive(Clone)]
struct ChatProjection {
    lines: Vec<std::rc::Rc<ChatProjectedLine>>,
    blocks: Vec<ChatBlock>,
}

type CachedActivity = (
    Vec<(
        String,
        Option<String>,
        Option<teshi_agent::backend::ToolActivityInfo>,
    )>,
    bool,
    String,
    ChatProjection,
);

/// Cached immutable message layouts, shared by drawing and pointer projection.
#[derive(Default)]
pub(crate) struct ChatLayoutCache {
    context: (usize, usize, bool, u64, Vec<usize>),
    activities: std::collections::HashMap<usize, CachedActivity>,
    entries: std::collections::HashMap<usize, (AiChatMessage, ChatProjection)>,
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

fn find_char_text(haystack: &[char], needle: &str, from: usize) -> Option<(usize, usize)> {
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
    block_text: &[char],
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
        || display_text.starts_with('○')
        || display_text.starts_with('▸')
        || display_text.starts_with('▾');
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

fn append_unselectable(out: &mut Vec<std::rc::Rc<ChatProjectedLine>>, lines: Vec<Line<'static>>) {
    out.extend(lines.into_iter().map(|line| {
        std::rc::Rc::new(ChatProjectedLine {
            line,
            source: None,
            action: None,
            image: None,
        })
    }));
}

/// Appends a label row per image mentioned in a user message, plus a box of
/// rows for its pixels when the terminal can draw them.
///
/// The label opens the image in the system viewer, so a terminal without a
/// pixel protocol still gives access to the full image.
fn append_message_images(
    out: &mut Vec<std::rc::Rc<ChatProjectedLine>>,
    app: &App,
    content: &str,
    width: usize,
) {
    let mentions = crate::chat_images::image_mentions(content, &app.project.root_dir);
    let body = rail_body_width(width);
    let font = app.inline_images.font_size();
    let user_bg = Style::default().bg(theme::CHAT_USER_BG);
    for (index, mention) in mentions.into_iter().enumerate() {
        let exists = mention.path.is_file();
        let name = crate::chat_images::display_name(&mention.path);
        let tag = crate::chat_images::placeholder(index + 1);
        let (label, style) = if exists {
            (format!("▣ {tag} {name}"), muted())
        } else {
            (
                format!("▣ {tag} {name} · file not found"),
                fg(theme::WARNING),
            )
        };
        let label = Line::styled(activity_summary(&label, body as usize), style).style(user_bg);
        let size = match (exists, font) {
            (true, Some(font)) => image::image_dimensions(&mention.path)
                .map_or((0, 0), |pixels| image_box(pixels, font, body)),
            _ => (0, 0),
        };
        let box_rows = if size.0 > 0 { size.1 } else { 0 };
        let mut lines = Vec::new();
        push_railed(
            &mut lines,
            std::iter::once(label)
                .chain((0..box_rows).map(|_| Line::default().style(user_bg)))
                .collect(),
            RailKind::User,
            width,
        );
        let path = std::rc::Rc::new(mention.path);
        out.extend(lines.into_iter().enumerate().map(|(index, line)| {
            let image = (index > 0).then(|| ImageRow {
                path: path.clone(),
                row: index as u16 - 1,
                size,
            });
            std::rc::Rc::new(ChatProjectedLine {
                line,
                source: None,
                action: (index == 0 && exists)
                    .then(|| crate::keymap::Action::OpenImageExternally((*path).clone())),
                image,
            })
        }));
    }
}

fn append_blank(out: &mut Vec<std::rc::Rc<ChatProjectedLine>>) {
    out.push(std::rc::Rc::new(ChatProjectedLine {
        line: Line::default(),
        source: None,
        action: None,
        image: None,
    }));
}

/// Separator rows between two agent-side blocks carry the guide.
///
/// Blocks are separated by a blank row so they read as one at a time, but the guide is
/// what groups a turn; leaving the separator empty cut the `│` column into dashes at every
/// message boundary. A gap beside a user block stays empty: `▌` opens a turn, and the
/// guide should not run through it.
fn carry_rail_through_gaps(out: &mut [std::rc::Rc<ChatProjectedLine>]) {
    let mut index = 0;
    while index < out.len() {
        if !out[index].line.spans.is_empty() {
            index += 1;
            continue;
        }
        let start = index;
        while index < out.len() && out[index].line.spans.is_empty() {
            index += 1;
        }
        if start == 0
            || index == out.len()
            || !starts_with_agent_rail(&out[start - 1])
            || !starts_with_agent_rail(&out[index])
        {
            continue;
        }
        for row in &mut out[start..index] {
            *row = std::rc::Rc::new(ChatProjectedLine {
                line: Line::from(Span::styled(QUIET_RAIL, fg(theme::BORDER))),
                source: None,
                action: None,
                image: None,
            });
        }
    }
}

fn starts_with_agent_rail(row: &ChatProjectedLine) -> bool {
    row.line
        .spans
        .first()
        .is_some_and(|span| span.content == QUIET_RAIL)
}

/// Add rendered rows and map visible body text to a width-independent block.
///
/// The block is the rendered plain-text projection of a message. Markdown
/// syntax is intentionally omitted from this projection, so selection copies
/// what the user can read rather than hidden formatting markers.
fn append_selectable_chunk(
    out: &mut Vec<std::rc::Rc<ChatProjectedLine>>,
    blocks: &mut Vec<ChatBlock>,
    lines: Vec<Line<'static>>,
    block_key: usize,
    block_text: String,
) {
    // Each message/group is appended once, and the streaming block has its own key.
    // Searching earlier blocks here makes cold layout quadratic in message count.
    blocks.push(ChatBlock {
        key: block_key,
        text: std::rc::Rc::new(block_text.clone()),
    });
    let source_chars: Vec<char> = block_text.chars().collect();
    let mut source_cursor = 0;
    for line in lines {
        let full_text = line_text(&line);
        let (display_start, selectable) = if let Some(body) = full_text
            .strip_prefix(RAIL)
            .or_else(|| full_text.strip_prefix(QUIET_RAIL))
        {
            (RAIL.chars().count(), body)
        } else {
            (0, full_text.as_str())
        };
        let candidate = selectable.trim_end_matches(' ');
        let source = find_display_source(&source_chars, candidate, source_cursor, display_start)
            .map(|(start, end, display_start)| ChatSourceRange {
                block: block_key,
                start,
                end,
                display_start,
                display_end: display_start + end.saturating_sub(start),
            });
        if let Some(source) = &source {
            source_cursor = source.end;
        }
        out.push(std::rc::Rc::new(ChatProjectedLine {
            line,
            source,
            action: None,
            image: None,
        }));
    }
}

fn block_text_for_message(msg: &AiChatMessage, base: &std::path::Path) -> String {
    if msg.source.as_deref() == Some("acp_reasoning") {
        plain_markdown_text(msg.reasoning_content.as_deref().unwrap_or_default())
    } else {
        plain_markdown_text(&shown_content(msg, base))
    }
}

/// Message text as the transcript shows it: a user message's image mentions
/// read as the same `[Image #N]` placeholders the draft showed, and the
/// images follow below the text.
fn shown_content<'a>(msg: &'a AiChatMessage, base: &std::path::Path) -> std::borrow::Cow<'a, str> {
    if msg.role == AiRole::User {
        let (shown, images) = crate::chat_images::collapse_mentions(&msg.content, base);
        if !images.is_empty() {
            return shown.into();
        }
    }
    msg.content.as_str().into()
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

    let title = chat_panel_title(app, area, placement);
    // A focused pane owns the bright outline. The live composer draws its own,
    // so the enclosing frame stays quiet while the user is typing and only
    // lights up when the chat pane is focused without the caret.
    let block = theme::panel_block(title, focused && !app.ai_input_focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_approval_badge(frame, app, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let input_active = focused && app.ai_input_focused;
    let queue_height = u16::from(!app.agent().chat.queue.is_empty() && inner.height >= 7);
    let alert_height = u16::from(needs_chat_status(app) && inner.height >= 6);
    let attachments = crate::chat_images::draft_attachments(
        &app.agent().input,
        &app.agent().input_images,
        &app.project.root_dir,
    );
    let attachment_height = u16::from(!attachments.is_empty() && inner.height >= 6);
    let dock_height = 1 + queue_height + alert_height + attachment_height;
    let input_width = composer_text_width(inner.width);
    let input_rows =
        visual_line_count_for_width(&app.agent().input, input_width).min((inner.height / 3).max(3));
    let input_height = (input_rows + 2).min(inner.height.saturating_sub(dock_height));
    let filter = slash_filter(app, placement);
    let suggestion_count = filter
        .as_deref()
        .map_or(0, |f| filtered_slash_commands(f).len() as u16);
    let suggestion_height = if suggestion_count > 0 {
        (suggestion_count + 1).min(inner.height.saturating_sub(dock_height + input_height + 1))
    } else {
        0
    };
    let chat_height = inner
        .height
        .saturating_sub(dock_height + suggestion_height + input_height);
    render_chat_history(
        frame,
        app,
        shrink_left(Rect::new(inner.x, inner.y, inner.width, chat_height), 1),
        placement,
    );
    let mut dock_y = inner.y + chat_height;
    if alert_height > 0 {
        render_chat_status(
            frame,
            app,
            shrink_left(Rect::new(inner.x, dock_y, inner.width, 1), 1),
        );
        dock_y += 1;
    }
    if queue_height > 0 {
        let count = app.agent().chat.queue.len();
        let state = if matches!(
            app.agent().status,
            AiStatus::Waiting | AiStatus::AwaitingApproval
        ) {
            "Enter queues another"
        } else {
            "/sendqueued to resume"
        };
        let preview = app
            .agent()
            .chat
            .queue
            .front()
            .map(|message| crate::chat_images::collapse_mentions(message, &app.project.root_dir).0)
            .unwrap_or_default();
        frame.render_widget(
            Paragraph::new(Line::styled(
                activity_summary(
                    &format!("Queued {count}: {preview} · /unqueue to edit · {state}"),
                    inner.width.saturating_sub(1) as usize,
                ),
                fg(theme::WARNING),
            )),
            Rect::new(inner.x + 1, dock_y, inner.width.saturating_sub(1), 1),
        );
        dock_y += 1;
    }
    render_chat_toolbar(
        frame,
        app,
        shrink_left(Rect::new(inner.x, dock_y, inner.width, 1), 1),
    );
    dock_y += 1;
    if attachment_height > 0 {
        render_chat_attachments(
            frame,
            app,
            &attachments,
            shrink_left(Rect::new(inner.x, dock_y, inner.width, 1), 1),
        );
        dock_y += 1;
    }
    if let Some(filter) = filter.as_deref()
        && suggestion_height > 0
    {
        render_slash_suggestions(
            frame,
            app,
            Rect::new(inner.x, dock_y, inner.width, suggestion_height),
            filter,
        );
    }
    render_chat_input(
        frame,
        app,
        Rect::new(
            inner.x,
            dock_y + suggestion_height,
            inner.width,
            input_height,
        ),
        input_active,
    );
}

/// Builds the top-border title and registers the session ID as a copy target.
///
/// Segments are admitted in priority order and only when they fit before the
/// approval badge, so the session ID outlives the agent name and sidebar hint
/// on narrow panels instead of being clipped mid-ID.
fn chat_panel_title(app: &mut App, area: Rect, placement: ChatPlacement) -> Line<'static> {
    // Block titles start after the left corner; the badge keeps a one-cell gap.
    let title_x = area.x + 1;
    let room = match approval_badge_label(app) {
        Some(label) => area.width.saturating_sub(label.width() as u16 + 4),
        None => area.width.saturating_sub(2),
    } as usize;

    let mut title = Line::from(Span::styled(
        " AI Chat ",
        fg(theme::FG).add_modifier(Modifier::BOLD),
    ));
    if let Some(id) = app.agent().session_id().map(str::to_owned) {
        let used = title.width();
        let id_with_gap = id.width() + " ".width();
        let prefix = ["· Session: ", "· "]
            .into_iter()
            .find(|prefix| used + prefix.width() + id_with_gap <= room);
        if let Some(prefix) = prefix {
            let rect = Rect::new(
                title_x + (used + prefix.width()) as u16,
                area.y,
                id.width() as u16,
                1,
            );
            let mut id_style = fg(theme::PRIMARY);
            if app
                .mouse_position
                .is_some_and(|(x, y)| rect.contains(ratatui::layout::Position::new(x, y)))
            {
                id_style = id_style.add_modifier(Modifier::REVERSED);
            }
            app.clickable_regions.push(ClickableRegion::ActionButton {
                rect,
                action: crate::keymap::Action::AiCopySessionId,
                modal: false,
            });
            title.push_span(Span::styled(prefix, muted()));
            title.push_span(Span::styled(id, id_style));
            title.push_span(Span::raw(" "));
        }
    }
    let mut optional = Vec::new();
    if !app.agent_registry.uses_builtin_fallback()
        && (app.agent().backend_selected || app.agent().backend.is_connected())
    {
        optional.push(format!("· Agent: {} ", app.active_agent_profile().name));
    }
    if placement == ChatPlacement::Tab && app.ai_sidebar_collapsed {
        optional.push("· chats hidden [Ctrl+g] ".to_string());
    }
    for segment in optional {
        if title.width() + segment.width() <= room {
            title.push_span(Span::styled(segment, muted()));
        }
    }
    title
}

/// One chip per image attached to the draft; clicking a chip removes it.
///
/// Chips that cannot fit collapse into a `+N` count so the row never wraps.
fn render_chat_attachments(
    frame: &mut Frame<'_>,
    app: &mut App,
    attachments: &[crate::chat_images::ImageMention],
    area: Rect,
) {
    let mut x = area.x;
    for (index, mention) in attachments.iter().enumerate() {
        let info = app.chat_attachments.info(&mention.path);
        let failed = info.is_err();
        let label = format!(
            " ▣ {} ✗ ",
            activity_summary(&crate::chat_images::chip_label(&mention.path, &info), 40)
        );
        let width = label.width() as u16;
        let remaining = attachments.len() - index;
        let overflow = format!("+{remaining}");
        // Keep room for the overflow count unless this is the last chip.
        let reserve = if remaining > 1 {
            overflow.width() as u16 + 1
        } else {
            0
        };
        if x.saturating_add(width + reserve) > area.right() {
            frame.render_widget(
                Paragraph::new(Line::styled(overflow, muted())),
                Rect::new(x, area.y, area.right().saturating_sub(x), 1),
            );
            return;
        }
        let rect = Rect::new(x, area.y, width, 1);
        let color = if failed { theme::ERROR } else { theme::PRIMARY };
        frame.render_widget(
            Paragraph::new(label).style(fg(color).bg(theme::CHAT_CHIP_BG)),
            rect,
        );
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect,
            action: crate::keymap::Action::AiRemoveAttachment(index),
            modal: false,
        });
        x += width + 1;
    }
}

/// Warnings and recovery remain visible; ordinary progress already lives in the transcript.
fn needs_chat_status(app: &App) -> bool {
    matches!(
        app.agent().status,
        AiStatus::AwaitingApproval | AiStatus::Error
    ) || app.agent().task_interrupted()
        || !app.agent().backend.is_connected()
        || !matches!(
            app.generation_stage,
            GenerationStage::Idle | GenerationStage::Complete
        )
}

/// Pick complete metric fields rather than clipping context counts or a warning marker.
fn chat_metrics(app: &App, width: usize) -> String {
    let agent = app.agent();
    let seconds = agent
        .task_started
        .map_or(agent.task_elapsed, |start| start.elapsed())
        .as_secs();
    let runtime = format!("{}m{:02}s", seconds / 60, seconds % 60);
    let limit = app.active_context_window();
    let (context, compact) = match agent.last_input_tokens {
        Some(tokens) => {
            let warning = if u64::from(tokens) * 10 >= u64::from(limit) * 7 {
                " !"
            } else {
                ""
            };
            (
                format!("ctx {tokens}/{limit}{warning}"),
                format!(
                    "ctx {}%{warning}",
                    u64::from(tokens) * 100 / u64::from(limit.max(1))
                ),
            )
        }
        None => ("ctx unknown".into(), "ctx ?".into()),
    };
    let speed = (agent.status == AiStatus::Waiting)
        .then(|| agent.chat.rate.per_second(std::time::Instant::now()))
        .flatten()
        .map(|rate| format!("~{rate:.0} tok/s"));
    let prefix = speed
        .as_ref()
        .map(|s| format!("{s} · "))
        .unwrap_or_default();
    let usage = token_usage_label(agent.total_input_tokens, agent.total_output_tokens);
    let mut candidates = Vec::new();
    if agent.total_input_tokens > 0 || agent.total_output_tokens > 0 {
        candidates.push(format!("{prefix}{runtime} · {context} · {usage}"));
    }
    candidates.extend([
        format!("{prefix}{runtime} · {context}"),
        format!("{prefix}{runtime} · {compact}"),
    ]);
    if let Some(speed) = speed {
        candidates.push(format!("{speed} · {compact}"));
    }
    candidates.extend([
        format!("{runtime} · {context}"),
        format!("{runtime} · {compact}"),
        compact,
        runtime,
    ]);
    candidates
        .into_iter()
        .find(|text| text.width() <= width)
        .unwrap_or_default()
}

/// Place selectable configuration chips beside a separate right-aligned metrics area.
fn render_chat_toolbar(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.is_empty() {
        return;
    }
    let min_controls = if area.width >= 75 { 36 } else { 18 };
    let metrics = chat_metrics(app, area.width.saturating_sub(min_controls + 1) as usize);
    let metrics_width = metrics.width() as u16;
    let controls_width = area
        .width
        .saturating_sub(metrics_width + u16::from(metrics_width > 0));
    if app.agent().backend.acp_controls().is_some() {
        crate::acp_controls::render_bar(frame, app, Rect::new(area.x, area.y, controls_width, 1));
    } else {
        render_native_controls(frame, app, Rect::new(area.x, area.y, controls_width, 1));
    }
    frame.render_widget(
        Paragraph::new(Line::styled(metrics, muted())).alignment(Alignment::Right),
        Rect::new(
            area.right().saturating_sub(metrics_width),
            area.y,
            metrics_width,
            1,
        ),
    );
}

fn render_native_controls(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let model = app.active_model_label.as_deref().unwrap_or("Select");
    let model = activity_summary(model, area.width.saturating_sub(27) as usize);
    let controls = [
        (
            if app.agent().backend_selected || app.agent().backend.is_connected() {
                " Agent Native ▾ "
            } else {
                " Agent ▾ "
            }
            .to_string(),
            crate::keymap::Action::AiOpenAgentSelector,
        ),
        (
            format!(" Model {model} ▾ "),
            crate::keymap::Action::ModelPanelOpen,
        ),
    ];
    let mut x = area.x;
    for (label, action) in controls {
        let width = label.width() as u16;
        if x.saturating_add(width) > area.right() {
            continue;
        }
        let rect = Rect::new(x, area.y, width, 1);
        frame.render_widget(
            Paragraph::new(label).style(fg(theme::PRIMARY).bg(theme::CHAT_CHIP_BG)),
            rect,
        );
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect,
            action,
            modal: false,
        });
        x += width + 1;
    }
    if app.agent().backend.is_connected()
        && app.agent().status == AiStatus::Idle
        && x + 5 <= area.right()
    {
        frame.render_widget(
            Paragraph::new("Ready").style(muted()),
            Rect::new(x, area.y, 5, 1),
        );
    }
}

/// ACP agents own their approval policy, so only built-in backends show the chip.
fn approval_badge_label(app: &App) -> Option<String> {
    (app.agent().backend.kind() != teshi_agent::backend::AgentBackendKind::Acp)
        .then(|| format!(" {} ", app.effective_approval_mode().display_name()))
}

/// Draws the approval-mode chip on the panel's top border and registers it as
/// a click target that opens the approval picker.
fn render_approval_badge(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let Some(label) = approval_badge_label(app) else {
        return;
    };
    let mode = app.effective_approval_mode();
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

    let mut style = if mode.requires_manual_approval() {
        muted().bg(theme::BLOCK_BG)
    } else {
        theme::chip(label.as_str(), chip_bg).style
    };
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

/// Replaces the trailing cells of a code block's language row with the copy
/// affordance. The label is drawn here, after selection mapping, so it never
/// becomes part of the copied chat text.
fn overlay_copy_label(line: &mut Line<'static>, width: u16, hovered: bool) {
    let label_width = crate::markdown::CODE_COPY_LABEL.width() as u16;
    let target = width.saturating_sub(label_width);
    let current = line.width() as u16;
    let mut rendered = std::mem::take(line);
    if current > target {
        rendered = super::truncate_line_to_cols(rendered, target);
    } else if current < target {
        rendered
            .spans
            .push(Span::raw(" ".repeat((target - current) as usize)));
    }
    let style = if hovered {
        fg(theme::PRIMARY).add_modifier(Modifier::BOLD)
    } else {
        fg(theme::MUTED)
    };
    rendered
        .spans
        .push(Span::styled(crate::markdown::CODE_COPY_LABEL, style));
    *line = rendered;
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
    let mut visible: Vec<Line<'static>> = (start..start.saturating_add(height).min(total))
        .map(|index| projection.highlighted_line(index, selection))
        .collect();
    let copy_width = (crate::markdown::CODE_COPY_LABEL.width() as u16).min(body.width);
    for (row, projected) in projection.lines.iter().skip(start).take(height).enumerate() {
        if let Some(action) = &projected.action {
            let copy_block = matches!(action, crate::keymap::Action::AiCopyCodeBlock { .. });
            let rect = Rect::new(
                if copy_block {
                    body.right().saturating_sub(copy_width)
                } else {
                    body.x
                },
                body.y + row as u16,
                if copy_block {
                    copy_width
                } else if matches!(action, crate::keymap::Action::AiToggleThought(_)) {
                    body.width.min(14)
                } else {
                    body.width
                },
                1,
            );
            if copy_block && let Some(line) = visible.get_mut(row) {
                let hovered = app
                    .mouse_position
                    .is_some_and(|(x, y)| rect.contains(ratatui::layout::Position::new(x, y)));
                overlay_copy_label(line, body.width, hovered);
            }
            app.clickable_regions.push(ClickableRegion::ActionButton {
                rect,
                action: action.clone(),
                modal: false,
            });
        }
    }
    frame.render_widget(Paragraph::new(Text::from(visible)), body);
    place_history_images(frame, app, &projection.lines, body, start);
    render_scrollbar(frame, app, scrollbar, total, height, start);
}

/// Places the visible rows of each image box, and notes boxes whose pixels
/// are not shown yet.
fn place_history_images(
    frame: &mut Frame<'_>,
    app: &mut App,
    lines: &[std::rc::Rc<ChatProjectedLine>],
    body: Rect,
    start: usize,
) {
    let x = body.x + RAIL.width() as u16;
    let end = start.saturating_add(body.height as usize).min(lines.len());
    let mut index = start;
    while index < end {
        let Some(image) = lines[index].image.clone() else {
            index += 1;
            continue;
        };
        let first = index;
        // A run is the visible, consecutive rows of one box.
        while index < end
            && lines[index].image.as_ref().is_some_and(|next| {
                std::rc::Rc::ptr_eq(&next.path, &image.path)
                    && usize::from(next.row) == usize::from(image.row) + (index - first)
            })
        {
            index += 1;
        }
        let area = Rect::new(
            x,
            body.y + (first - start) as u16,
            image.size.0.min(body.right().saturating_sub(x)),
            (index - first) as u16,
        );
        let note = match app
            .inline_images
            .place(frame, &image.path, image.size, area, image.row)
        {
            crate::image_view::InlineState::Shown => None,
            crate::image_view::InlineState::Loading => Some(("Loading image…".into(), muted())),
            crate::image_view::InlineState::Failed(reason) => {
                Some((format!("Cannot show image: {reason}"), fg(theme::WARNING)))
            }
            crate::image_view::InlineState::Placeholder => {
                Some(("Scroll to show the image".into(), muted()))
            }
        };
        if let Some((text, style)) = note {
            frame.render_widget(
                Paragraph::new(Line::styled(
                    activity_summary(&text, area.width as usize),
                    style,
                )),
                Rect::new(area.x, area.y, area.width, 1),
            );
        }
    }
}

fn render_scrollbar(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    total: usize,
    height: usize,
    start: usize,
) {
    if total <= height || area.width == 0 || height == 0 {
        app.chat_scrollbar = None;
        return;
    }
    let bar = crate::app::ScrollbarBar {
        rect: area,
        axis: crate::theme::ScrollAxis::Vertical,
        total,
    };
    app.chat_scrollbar = Some(bar);
    let state =
        crate::app::scrollbar_state(app.mouse_position, &bar, app.chat_scrollbar_grab.is_some());
    let lines = theme::scrollbar_lines(
        total,
        height,
        start,
        crate::theme::ScrollAxis::Vertical,
        state,
    );
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
        .map(|line| line.line.clone())
        .collect()
}

fn build_chat_projection(
    app: &App,
    width: usize,
    view_height: usize,
    placement: ChatPlacement,
) -> ChatProjection {
    let mut out: Vec<std::rc::Rc<ChatProjectedLine>> = Vec::new();
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
        if !agent.backend.is_connected() {
            let hint = if agent.backend_selected {
                "Agent preselected; not connected. Focus the input to start, or use /agent."
            } else {
                "No agent selected. Use /agent to choose, or focus the input to use Teshi Native."
            };
            append_unselectable(
                &mut out,
                wrap_line_to_width(Line::styled(hint, muted()), width),
            );
            append_blank(&mut out);
        }
        if agent.backend_kind() == teshi_agent::backend::AgentBackendKind::Native
            && !crate::llm::is_configured()
        {
            let note =
                "Note: Run 'teshi auth login' or set TESHI_LLM_API_KEY to enable AI responses.";
            append_unselectable(
                &mut out,
                wrap_line_to_width(Line::styled(note, fg(theme::WARNING)), width),
            );
        }
    }

    let messages = agent.messages();
    let completed: Vec<usize> = agent
        .completed_answers
        .iter()
        .copied()
        .filter(|index| {
            messages
                .get(*index)
                .is_some_and(|message| message.role == AiRole::Assistant)
        })
        .collect();
    // Every finished answer keeps its green rail for good: the mark records the
    // turn that ended, so later turns must not repaint it as a quiet guide.
    let mut cache = app.chat_layout_cache.borrow_mut();
    let context = (
        app.selected_agent,
        width,
        agent.activity_details_expanded,
        agent.chat.revision,
        completed.clone(),
    );
    if cache.context != context {
        cache.entries.clear();
        cache.activities.clear();
        cache.context = context;
    }
    cache.entries.retain(|index, _| *index < messages.len());
    cache.activities.retain(|index, _| *index < messages.len());
    let last_user = messages
        .iter()
        .rposition(|message| message.role == AiRole::User);
    let turns = crate::chat_state::turns(messages);
    let mut turn_index = 0;
    let mut folded_answer = None;
    let mut message_index = 0;
    while let Some(msg) = messages.get(message_index) {
        while turns
            .get(turn_index)
            .is_some_and(|turn| message_index >= turn.end)
        {
            turn_index += 1;
        }
        if let Some(turn) = turns.get(turn_index)
            && message_index == turn.start + 1
            && let Some(answer) = turn.answer
            && (turn_index + 1 < turns.len()
                || (agent.status == AiStatus::Idle
                    && !agent.task_interrupted()
                    && !matches!(
                        agent.last_finish_reason.as_deref(),
                        Some("cancelled" | "length" | "max_tokens")
                    )))
            && (answer > message_index
                || messages[answer]
                    .reasoning_content
                    .as_ref()
                    .is_some_and(|s| !s.is_empty()))
        {
            let expanded = agent
                .chat
                .turns
                .get(&turn.start)
                .copied()
                .unwrap_or(agent.activity_details_expanded);
            let summary = if messages[turn.start].source.as_deref() == Some("compacted") {
                "Activity".into()
            } else {
                crate::chat_state::process_summary(&messages[turn.start + 1..turn.end])
            };
            let label = fit_activity_header(
                if expanded { "▾ " } else { "▸ " },
                &summary,
                rail_body_width(width) as usize,
            );
            out.push(std::rc::Rc::new(ChatProjectedLine {
                line: Line::from(vec![
                    Span::styled(QUIET_RAIL, fg(theme::BORDER)),
                    Span::styled(label, muted()),
                ]),
                source: None,
                action: Some(crate::keymap::Action::AiToggleTurn(turn.start)),
                image: None,
            }));
            append_blank(&mut out);
            if !expanded {
                message_index = answer;
                folded_answer = Some(answer);
                if answer > turn.start + 1 {
                    continue;
                }
            }
        }
        if msg.source.as_deref() == Some("acp_activity") {
            let group_end = messages[message_index..]
                .iter()
                .take_while(|message| message.source.as_deref() == Some("acp_activity"))
                .count()
                + message_index;
            let group: Vec<&AiChatMessage> = messages[message_index..group_end].iter().collect();
            let active = matches!(agent.status, AiStatus::Waiting | AiStatus::AwaitingApproval)
                && last_user.is_none_or(|index| index < group_end);
            let animated = active
                && group
                    .iter()
                    .any(|message| message.reasoning_content.as_deref() == Some("in_progress"));
            let animation = if animated { tool_spinner_frame() } else { "" };
            if let Some((previous, previous_active, previous_animation, projection)) =
                cache.activities.get(&message_index)
                && *previous_active == active
                && previous_animation == animation
                && previous.len() == group.len()
                && previous
                    .iter()
                    .zip(&group)
                    .all(|((content, status, details), message)| {
                        content == &message.content
                            && status == &message.reasoning_content
                            && details == &message.activity
                    })
            {
                out.extend(projection.lines.iter().cloned());
                blocks.extend(projection.blocks.iter().cloned());
                message_index = group_end;
                continue;
            }
            let line_start = out.len();
            let block_start = blocks.len();
            let mut chunk = Vec::new();
            push_activity_group(
                &mut chunk,
                &group,
                width,
                agent.activity_details_expanded || agent.chat.groups.contains(&message_index),
                active,
            );
            let source = group
                .iter()
                .map(|message| plain_markdown_text(&message.content))
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            // A press on the header toggles the group, so it cannot start a
            // selection. Map the copyable titles onto the detail rows instead;
            // otherwise the header claims them and no row can be selected.
            let mut rows = chunk.into_iter();
            if let Some(header) = rows.next() {
                out.push(std::rc::Rc::new(ChatProjectedLine {
                    line: header,
                    source: None,
                    action: Some(crate::keymap::Action::AiToggleTools(message_index)),
                    image: None,
                }));
            }
            append_selectable_chunk(&mut out, &mut blocks, rows.collect(), message_index, source);
            append_blank(&mut out);
            cache.activities.insert(
                message_index,
                (
                    group
                        .iter()
                        .map(|message| {
                            (
                                message.content.clone(),
                                message.reasoning_content.clone(),
                                message.activity.clone(),
                            )
                        })
                        .collect(),
                    active,
                    animation.to_owned(),
                    ChatProjection {
                        lines: out[line_start..].to_vec(),
                        blocks: blocks[block_start..].to_vec(),
                    },
                ),
            );
            message_index = group_end;
            continue;
        }
        // Tool-call rows depend on live approval/result state and animation.
        let live_thought = agent.status == AiStatus::Waiting
            && msg.source.as_deref() == Some("acp_reasoning")
            && message_index + 1 == messages.len();
        let cacheable = folded_answer != Some(message_index)
            && !live_thought
            && msg.tool_calls.as_ref().is_none_or(|calls| calls.is_empty());
        if cacheable
            && let Some((previous, projection)) = cache.entries.get(&message_index)
            && previous.role == msg.role
            && previous.content == msg.content
            && previous.source == msg.source
            && previous.reasoning_content == msg.reasoning_content
            && previous.tool_call_id == msg.tool_call_id
        {
            out.extend(projection.lines.iter().cloned());
            blocks.extend(projection.blocks.iter().cloned());
            message_index += 1;
            continue;
        }
        let line_start = out.len();
        let block_start = blocks.len();
        let mut chunk = Vec::new();
        let copy_rows = match msg.role {
            AiRole::Tool => {
                push_tool_result(&mut chunk, msg, width);
                Vec::new()
            }
            AiRole::User | AiRole::Assistant => push_message(
                &mut chunk,
                app,
                msg,
                width,
                MessageDisplay {
                    expanded: agent.activity_details_expanded
                        || agent.chat.thoughts.contains(&message_index),
                    show_reasoning: folded_answer != Some(message_index),
                    live: live_thought,
                    completed: completed.contains(&message_index),
                },
            ),
        };
        append_selectable_chunk(
            &mut out,
            &mut blocks,
            chunk,
            message_index,
            block_text_for_message(msg, &app.project.root_dir),
        );
        for (row, block_index) in copy_rows {
            if let Some(line) = out.get_mut(line_start + row) {
                std::rc::Rc::make_mut(line).action = Some(crate::keymap::Action::AiCopyCodeBlock {
                    message_index,
                    block_index,
                });
            }
        }
        if msg.role == AiRole::User {
            append_message_images(&mut out, app, &msg.content, width);
        }
        if folded_answer != Some(message_index)
            && (msg.source.as_deref() == Some("acp_reasoning")
                || msg
                    .reasoning_content
                    .as_ref()
                    .is_some_and(|s| !s.is_empty()))
            && msg.source.as_deref() != Some("acp_activity")
            && let Some(row) = out.get_mut(line_start)
        {
            std::rc::Rc::make_mut(row).action =
                Some(crate::keymap::Action::AiToggleThought(message_index));
        }
        append_blank(&mut out);
        if cacheable {
            cache.entries.insert(
                message_index,
                (
                    msg.clone(),
                    ChatProjection {
                        lines: out[line_start..].to_vec(),
                        blocks: blocks[block_start..].to_vec(),
                    },
                ),
            );
        } else {
            cache.entries.remove(&message_index);
        }
        message_index += 1;
    }

    let mut pending = Vec::new();
    push_pending_diff(&mut pending, app, width, view_height);
    append_unselectable(&mut out, pending);

    if !agent.backend.partial_reasoning().is_empty() {
        let mut reasoning = Vec::new();
        push_thinking_state(
            &mut reasoning,
            agent.backend.partial_reasoning(),
            width,
            agent.activity_details_expanded || agent.chat.thoughts.contains(&usize::MAX),
            true,
        );
        let first = out.len();
        append_unselectable(&mut out, reasoning);
        if let Some(row) = out.get_mut(first) {
            std::rc::Rc::make_mut(row).action =
                Some(crate::keymap::Action::AiToggleThought(usize::MAX));
        }
    }

    if !agent.partial_response().is_empty() {
        let mut body = vec![role_header(
            AiRole::Assistant,
            Some("agent_progress"),
            false,
        )];
        body.extend(render_markdown_width(
            agent.partial_response(),
            rail_body_width(width),
        ));
        // The blinking cursor marks an actively running turn. A finished turn
        // must never leave it at the end of the transcript.
        if agent.status == AiStatus::Waiting {
            let cursor = Span::styled(
                STREAM_CURSOR,
                fg(theme::ACCENT).add_modifier(Modifier::SLOW_BLINK),
            );
            match body.last_mut() {
                Some(last) => last.spans.push(cursor),
                None => body.push(Line::from(cursor)),
            }
        }
        let mut chunk = Vec::new();
        push_railed(&mut chunk, body, RailKind::Guide, width);
        append_selectable_chunk(
            &mut out,
            &mut blocks,
            chunk,
            usize::MAX,
            plain_markdown_text(agent.partial_response()),
        );
        append_blank(&mut out);
    }

    let shows_thought = !agent.backend.partial_reasoning().is_empty()
        || messages
            .last()
            .is_some_and(|m| m.source.as_deref() == Some("acp_reasoning"));
    let shows_tool = messages
        .iter()
        .rev()
        .take_while(|m| m.role != AiRole::User)
        .any(|m| {
            m.source.as_deref() == Some("acp_activity")
                && matches!(
                    m.reasoning_content.as_deref(),
                    Some("in_progress" | "pending")
                )
        });
    if agent.status == AiStatus::Waiting
        && agent.partial_response().is_empty()
        && !shows_thought
        && !shows_tool
        && agent
            .chat
            .last_activity
            .is_some_and(|time| time.elapsed() >= std::time::Duration::from_millis(350))
    {
        append_unselectable(
            &mut out,
            vec![Line::from(vec![
                Span::styled(QUIET_RAIL, fg(theme::BORDER)),
                Span::styled(format!("{} ", thinking_pulse_frame()), fg(theme::ACCENT)),
                Span::styled("Waiting for agent…", muted()),
            ])],
        );
    }
    let mut permission = Vec::new();
    push_acp_permission_prompt(&mut permission, app, width);
    append_unselectable(&mut out, permission);
    carry_rail_through_gaps(&mut out);
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

fn role_header(role: AiRole, source: Option<&str>, completed: bool) -> Line<'static> {
    let (label, color) = match role {
        AiRole::User => ("You", theme::ACCENT),
        _ if source == Some("agent_progress") => ("Teshi", theme::MUTED),
        _ => ("Teshi", theme::SUCCESS),
    };
    let mut spans = Vec::with_capacity(3);
    if let Some(source) = source.filter(|source| *source != "agent_progress") {
        spans.push(Span::styled(format!("[{source}] "), muted()));
    }
    spans.push(Span::styled(label, fg(color).add_modifier(Modifier::BOLD)));
    if completed {
        spans.push(Span::styled(
            " ✓",
            fg(theme::SUCCESS).add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}

/// Which left rail a block carries.
///
/// The rail marks the column, not the block state: every row of a block shares one rail
/// so a turn reads as one column. Only two boundaries change it — the accent bar opens a
/// user turn, and the green bar marks every answer that finished.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RailKind {
    /// Accent bar that opens a user turn.
    User,
    /// Green bar on a finished answer.
    Answer,
    /// Quiet thin guide for thinking, tools and unfinished replies.
    Guide,
}

impl RailKind {
    fn glyph(self) -> &'static str {
        match self {
            RailKind::Guide => QUIET_RAIL,
            RailKind::User | RailKind::Answer => RAIL,
        }
    }

    fn color(self) -> ratatui::style::Color {
        match self {
            RailKind::User => theme::ACCENT,
            RailKind::Answer => theme::SUCCESS,
            RailKind::Guide => theme::BORDER,
        }
    }
}

/// Wraps `lines` to the space right of the rail and prefixes every row with `rail`,
/// so wrapped continuation rows stay visually grouped.
/// Returns the output row where each input line starts, so callers can attach
/// row-level hitboxes after wrapping.
fn push_railed(
    out: &mut Vec<Line<'static>>,
    lines: Vec<Line<'static>>,
    rail: RailKind,
    width: usize,
) -> Vec<usize> {
    let body_width = rail_body_width(width) as usize;
    let rail = Span::styled(rail.glyph(), fg(rail.color()));
    let mut starts = Vec::with_capacity(lines.len());
    for line in lines {
        starts.push(out.len());
        for row in wrap_line_to_width(line, body_width) {
            let mut spans = Vec::with_capacity(row.spans.len() + 1);
            spans.push(rail.clone());
            spans.extend(row.spans);
            out.push(Line::from(spans));
        }
    }
    starts
}

/// How one message is drawn: its thinking open or folded, its reasoning shown, its
/// stream still running, and whether it finished a turn.
#[derive(Clone, Copy)]
struct MessageDisplay {
    expanded: bool,
    show_reasoning: bool,
    live: bool,
    completed: bool,
}

fn push_message(
    out: &mut Vec<Line<'static>>,
    app: &App,
    msg: &AiChatMessage,
    width: usize,
    display: MessageDisplay,
) -> Vec<(usize, usize)> {
    let entry = out.len();
    if msg.source.as_deref() == Some("acp_reasoning") {
        push_thinking_state(
            out,
            msg.reasoning_content.as_deref().unwrap_or_default(),
            width,
            display.expanded,
            display.live,
        );
        return Vec::new();
    }
    if let Some(reasoning) = &msg.reasoning_content
        && !reasoning.is_empty()
        && display.show_reasoning
    {
        push_thinking_state(out, reasoning, width, display.expanded, display.live);
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
        display.completed,
    )];
    let content = shown_content(msg, &app.project.root_dir);
    let mut code_blocks = Vec::new();
    if !content.is_empty() {
        let (lines, blocks) = crate::markdown::render_markdown_width_with_code_blocks(
            &content,
            rail_body_width(width),
        );
        body.extend(lines);
        code_blocks = blocks;
    }
    if let Some(tool_calls) = &msg.tool_calls {
        body.extend(tool_calls.iter().map(|tc| tool_call_line(app, tc)));
    }
    let rail = match msg.role {
        AiRole::User => RailKind::User,
        _ if display.completed && !progress => RailKind::Answer,
        _ => RailKind::Guide,
    };
    if msg.role == AiRole::User {
        for line in &mut body {
            line.style = line.style.bg(theme::CHAT_USER_BG);
        }
    }
    let starts = push_railed(out, body, rail, width);
    // The role header occupies body row 0; the markdown rows follow it.
    code_blocks
        .into_iter()
        .enumerate()
        .filter(|(_, block)| block.copy_label && block.line + 1 < starts.len())
        .map(|(block_index, block)| {
            let row = starts[block.line + 1].saturating_sub(entry);
            (row, block_index)
        })
        .collect()
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

/// Keep failure and interruption outcomes visible before clipping less useful details.
fn fit_activity_header(prefix: &str, summary: &str, width: usize) -> String {
    let (body, tail) = summary.split(" · ").fold(
        (String::new(), String::new()),
        |(mut body, mut tail), part| {
            if part.ends_with(" failed") || part.ends_with(" interrupted") || part == "Failed" {
                tail.push_str(&format!(" · {part}"));
            } else {
                if !body.is_empty() {
                    body.push_str(" · ");
                }
                body.push_str(part);
            }
            (body, tail)
        },
    );
    let room = width.saturating_sub(tail.width());
    let head = activity_summary(&format!("{prefix}{body}"), room);
    activity_summary(&format!("{head}{tail}"), width)
}

#[cfg(test)]
fn push_thinking(out: &mut Vec<Line<'static>>, text: &str, width: usize, expanded: bool) {
    push_thinking_state(out, text, width, expanded, false);
}

fn push_thinking_state(
    out: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    expanded: bool,
    live: bool,
) {
    let available = rail_body_width(width) as usize;
    let header = if expanded {
        "▾ Thinking"
    } else {
        "▸ Thinking: "
    };
    if expanded {
        let mut body = vec![Line::styled(activity_summary(header, available), muted())];
        body.extend(
            text.lines()
                .map(|line| Line::styled(line.to_owned(), muted())),
        );
        push_railed(out, body, RailKind::Guide, width);
        return;
    }
    // Read only a bounded slice even when a provider streams a very long thought.
    let preview = if live {
        text.chars()
            .rev()
            .take(512)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>()
    } else {
        text.chars().take(512).collect()
    };
    let room = available.saturating_sub(header.width());
    let normalized = preview.split_whitespace().collect::<Vec<_>>().join(" ");
    let preview = if live && normalized.width() > room && room > 0 {
        let mut cells = 1;
        let tail: String = normalized
            .chars()
            .rev()
            .take_while(|ch| {
                let size = ch.width().unwrap_or(0);
                if cells + size > room {
                    false
                } else {
                    cells += size;
                    true
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("…{tail}")
    } else {
        activity_summary(&normalized, room)
    };
    let mut heading = activity_summary(header, available);
    if !preview.is_empty() {
        heading.push(' ');
    }
    let mut spans = vec![Span::styled(heading, muted())];
    let phase = frame_index(120, preview.chars().count().max(1) + 8);
    for (index, ch) in preview.chars().enumerate() {
        let style = if live && index <= phase && phase - index < 6 {
            fg(theme::ACCENT)
        } else {
            muted()
        };
        spans.push(Span::styled(ch.to_string(), style));
    }
    push_railed(out, vec![Line::from(spans)], RailKind::Guide, width);
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
    let summary = if group.len() == 1 {
        let message = group[0];
        let state = match message.reasoning_content.as_deref() {
            Some("completed") => "Done",
            Some("failed") => "Failed",
            Some("in_progress") if active => "Running",
            _ if !active => "Unconfirmed",
            _ => "Pending",
        };
        format!("{state}  {}", crate::chat_state::external_summary(message))
    } else {
        crate::chat_state::process_summary(&group.iter().map(|m| (*m).clone()).collect::<Vec<_>>())
    };
    let mut body = vec![Line::styled(
        fit_activity_header(
            if expanded { "▾ " } else { "▸ " },
            &summary,
            rail_body_width(width) as usize,
        ),
        fg(color),
    )];
    if !expanded {
        if active
            && group.len() > 1
            && let Some(message) = group.iter().rev().find(|m| {
                matches!(
                    m.reasoning_content.as_deref(),
                    Some("in_progress" | "pending")
                )
            })
        {
            body.push(Line::styled(
                activity_summary(
                    &format!(
                        "{} {}",
                        tool_spinner_frame(),
                        crate::chat_state::external_summary(message)
                    ),
                    rail_body_width(width) as usize,
                ),
                fg(theme::WARNING),
            ));
        }
        push_railed(out, body, RailKind::Guide, width);
        return;
    }
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
            &crate::chat_state::external_summary(msg),
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
    push_railed(out, body, RailKind::Guide, width);
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
        Span::styled(
            format!(" {}", crate::chat_state::native_summary(tc)),
            muted(),
        ),
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
            Span::styled(QUIET_RAIL, fg(theme::BORDER)),
            Span::styled(icon, fg(color).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" Tool ({tool_id})"), muted()),
        ]),
        width,
    ));

    // Tool output is supporting detail: default its text to the muted color
    // while keeping any explicit Markdown colors (code, headings).
    let indent = QUIET_RAIL.width() + TOOL_RESULT_INDENT.width();
    let body_width = width.saturating_sub(indent);
    let body = render_markdown_width(&msg.content, body_width.min(u16::MAX as usize) as u16);
    for line in body {
        for row in wrap_line_to_width(line, body_width) {
            let mut spans = vec![
                Span::styled(QUIET_RAIL, fg(theme::BORDER)),
                Span::raw(TOOL_RESULT_INDENT),
            ];
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

/// Text for the one-row error status.
///
/// Native and ACP turn failures are recorded as `AI error:`; a missing backend
/// is `AI backend unavailable:`. A leftover status from an unrelated action
/// must not replace the failure, but neither may a recorded AI or ACP failure
/// be rewritten as an API-key hint.
fn chat_error_label(status: &str) -> String {
    let trimmed = status.trim();
    if trimmed.starts_with("AI error:")
        || trimmed.starts_with("AI backend unavailable:")
        || trimmed.starts_with("ACP ")
        || trimmed.starts_with("Could not reconnect ACP")
    {
        trimmed.to_string()
    } else {
        "AI error — check TESHI_LLM_API_KEY and your network connection.".to_string()
    }
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
        let status = match app.agent().last_stop_detail.as_deref() {
            Some(detail) => format!("Task interrupted ({reason}): {detail} · /retry"),
            None => format!("Task interrupted ({reason}) · /retry"),
        };
        frame.render_widget(
            Paragraph::new(Line::styled(status, fg(theme::WARNING))),
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
        AiStatus::Error => chat_error_label(&app.status),
        AiStatus::Idle if agent.backend.is_connected() => "Ready".into(),
        AiStatus::Idle if agent.backend_selected => "Not connected · /agent".into(),
        AiStatus::Idle => "No agent selected · /agent".into(),
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
        AiStatus::Idle => left.push(Span::styled(status_text, muted())),
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

    frame.render_widget(Paragraph::new(Line::from(left)), area);
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

/// Right-side controls disappear as a unit in very narrow composers.
fn composer_button_width(inner_width: u16) -> u16 {
    if inner_width >= 26 { 9 } else { 0 }
}

fn composer_text_width(outer_width: u16) -> u16 {
    let inner = outer_width.saturating_sub(2);
    inner
        .saturating_sub(
            2 + composer_button_width(inner) + u16::from(composer_button_width(inner) > 0),
        )
        .max(1)
}

/// Draws `member @ branch` on the composer's top border.
///
/// The label is display-only: it tells the user which workspace member this chat
/// is anchored to without consuming a toolbar slot.
fn render_workspace_badge(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let Some((badge, member_count)) = app
        .workspace_context()
        .map(|context| (context.badge(), context.member_count))
    else {
        return;
    };
    let label = format!(" {badge} ");
    let width = label.width() as u16;
    if width + 3 > area.width {
        return;
    }
    let rect = Rect::new(area.x + 2, area.y, width, 1);
    frame.render_widget(Paragraph::new(label).style(muted()), rect);
    if member_count > 1 {
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect,
            action: crate::keymap::Action::WorkspaceNextMember,
            modal: false,
        });
    }
}

fn render_chat_input(frame: &mut Frame<'_>, app: &mut App, area: Rect, active: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(fg(if active {
            theme::PRIMARY
        } else {
            theme::BORDER
        }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    render_workspace_badge(frame, app, area);
    let button_width = composer_button_width(inner.width);
    let text_width = composer_text_width(area.width).min(inner.width.saturating_sub(2));
    let text_x = inner.x + 2.min(inner.width);
    let text_area = Rect::new(text_x, inner.y, text_width, inner.height);
    app.clickable_regions.push(ClickableRegion::ChatInput {
        rect: Rect::new(
            area.x,
            area.y,
            area.width
                .saturating_sub(button_width + u16::from(button_width > 0)),
            area.height,
        ),
    });
    frame.render_widget(
        Paragraph::new("›").style(fg(if active { theme::ACCENT } else { theme::MUTED })),
        Rect::new(inner.x, inner.y, 1, 1),
    );
    let agent = app.agent();
    let (rows, cursor) = input_rows(&agent.input, agent.input_cursor, text_width);
    let scroll = cursor.0.saturating_sub(inner.height - 1);
    let display = if agent.input.is_empty() {
        let placeholder = if active {
            if matches!(agent.status, AiStatus::Waiting | AiStatus::AwaitingApproval) {
                "Type a follow-up · Enter queues it"
            } else {
                "Type your message..."
            }
        } else {
            ""
        };
        Text::from(Line::styled(placeholder, muted()))
    } else {
        Text::from(
            rows.into_iter()
                .skip(usize::from(scroll))
                .collect::<Vec<_>>(),
        )
    };
    frame.render_widget(Paragraph::new(display), text_area);
    if active && text_width > 0 {
        let position = (text_x + cursor.1, inner.y + cursor.0 - scroll);
        let under = agent.input.chars().nth(agent.input_cursor);
        paint_block_cursor(frame, text_area, position, under);
        // The hardware cursor stays at the caret so OS IME candidate windows
        // anchor to the typed text even when the terminal hides it.
        frame.set_cursor_position(position);
    }
    if button_width > 0 {
        let busy = matches!(agent.status, AiStatus::Waiting | AiStatus::AwaitingApproval);
        let label = if busy { " Queue ↵ " } else { " Send ↵  " };
        let action = if agent.input.is_empty() {
            crate::keymap::Action::AiFocusInput
        } else {
            crate::keymap::Action::AiSendMessage
        };
        let button = Rect::new(
            inner.right() - button_width,
            inner.bottom() - 1,
            button_width,
            1,
        );
        frame.render_widget(
            Paragraph::new(label).style(fg(theme::PRIMARY).bg(theme::CHAT_CHIP_BG)),
            button,
        );
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect: button,
            action,
            modal: false,
        });
    }
}

/// Paints a solid block over the composer caret.
///
/// Many terminals draw the hardware cursor as a thin or invisible bar, which
/// leaves an empty focused composer indistinguishable from an idle one. The
/// block covers both cells of a wide character so it never splits a glyph.
fn paint_block_cursor(frame: &mut Frame<'_>, area: Rect, (x, y): (u16, u16), under: Option<char>) {
    let cells = under
        .filter(|ch| !ch.is_control())
        .and_then(UnicodeWidthChar::width)
        .unwrap_or(1)
        .max(1) as u16;
    let style = Style::default().fg(theme::CURSOR_FG).bg(theme::CURSOR_BG);
    let buffer = frame.buffer_mut();
    for column in x..x.saturating_add(cells).min(area.right()) {
        if area.contains((column, y).into()) {
            buffer[(column, y)].set_style(style);
        }
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
pub(crate) fn wrap_line_to_width(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
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
    fn message(role: AiRole, content: &str, source: Option<&str>) -> AiChatMessage {
        AiChatMessage {
            role,
            content: content.into(),
            source: source.map(str::to_owned),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            activity: None,
        }
    }

    #[test]
    fn chat_error_label_keeps_the_recorded_failure() {
        let cases = [
            (
                "AI error: background LLM thread has exited",
                "AI error: background LLM thread has exited",
            ),
            (
                "AI error: ACP prompt failed: rate limit",
                "AI error: ACP prompt failed: rate limit",
            ),
            (
                "  AI backend unavailable: connection reset  ",
                "AI backend unavailable: connection reset",
            ),
            (
                "ACP permission request is no longer pending",
                "ACP permission request is no longer pending",
            ),
            (
                "Could not reconnect ACP for profile Cursor: closed",
                "Could not reconnect ACP for profile Cursor: closed",
            ),
        ];
        for (status, expected) in cases {
            assert_eq!(chat_error_label(status), expected, "{status}");
        }
        let generic = "AI error — check TESHI_LLM_API_KEY and your network connection.";
        assert_eq!(chat_error_label("Switched to model: grok-4.7"), generic);
        assert_eq!(chat_error_label("   "), generic);
    }

    #[test]
    fn expanded_tool_titles_are_selectable_below_the_toggle_header() {
        let (mut app, _project, _store) = activity_test_app();
        let mut tool = message(AiRole::Assistant, "READ_PROJECT_TOOL", Some("acp_activity"));
        tool.reasoning_content = Some("completed".into());
        app.agents[0].activity_details_expanded = true;
        app.agents[0].messages_mut().push(tool);

        let projection = build_chat_projection(&app, 80, 40, ChatPlacement::Tab);
        let rows: Vec<_> = projection
            .lines
            .iter()
            .filter(|row| line_text(&row.line).contains("READ_PROJECT_TOOL"))
            .collect();
        assert_eq!(rows.len(), 2, "expected a header and a detail row");
        assert!(
            rows[0].action.is_some() && rows[0].source.is_none(),
            "the header toggles the group instead of starting a selection"
        );
        assert!(
            rows[1].source.is_some(),
            "the detail row carries the copyable title"
        );
    }

    #[test]
    fn completed_turns_fold_independently_and_keep_failed_call_counts() {
        let (mut app, _project, _store) = activity_test_app();
        let mut tool = message(AiRole::Assistant, "hidden_tool_title", Some("acp_activity"));
        tool.reasoning_content = Some("failed".into());
        tool.activity = Some(teshi_agent::backend::ToolActivityInfo {
            kind: Some("read".into()),
            input: Some(serde_json::json!({"path":"src/hidden.rs"})),
            locations: vec![],
        });
        app.agents[0].messages_mut().extend([
            message(AiRole::User, "First question", None),
            tool.clone(),
            message(AiRole::Assistant, "First answer", None),
            message(AiRole::User, "Second question", None),
            tool,
            message(AiRole::Assistant, "Second answer", None),
        ]);
        let folded = row_texts(&build_chat_lines(&app, 64, 40, ChatPlacement::Tab)).join("\n");
        assert!(
            folded.contains("First answer") && folded.contains("Second answer"),
            "{folded}"
        );
        assert_eq!(folded.matches("1 failed").count(), 2, "{folded}");
        assert!(!folded.contains("src/hidden.rs"));
        app.ai_input_focused = true;
        app.handle_action(crate::keymap::Action::AiToggleTurn(0))
            .unwrap();
        assert!(
            app.ai_input_focused,
            "opening process must keep the composer's focus"
        );
        let opened = row_texts(&build_chat_lines(&app, 64, 40, ChatPlacement::Tab)).join("\n");
        assert_eq!(opened.matches("Read src/hidden.rs").count(), 1, "{opened}");
        assert!(opened.contains("First answer") && opened.contains("Second answer"));
        for width in [22, 36, 64] {
            let label = fit_activity_header(
                "▸ ",
                "12 tool calls: 8 reads · 4 commands · 2 failed",
                width,
            );
            assert!(label.contains("2 failed"), "{label}");
            assert!(label.width() <= width);
        }
    }

    #[test]
    fn a_final_answer_with_embedded_reasoning_remains_visible_when_folded() {
        let (mut app, _project, _store) = activity_test_app();
        let mut answer = message(AiRole::Assistant, "FINAL_ANSWER", None);
        answer.reasoning_content = Some("FIRST_THOUGHT\nSECOND_THOUGHT".into());
        app.agents[0]
            .messages_mut()
            .extend([message(AiRole::User, "Question", None), answer]);
        let folded = row_texts(&build_chat_lines(&app, 60, 30, ChatPlacement::Tab)).join("\n");
        assert!(
            folded.contains("FINAL_ANSWER") && !folded.contains("FIRST_THOUGHT"),
            "{folded}"
        );
        app.handle_action(crate::keymap::Action::AiToggleTurn(0))
            .unwrap();
        app.handle_action(crate::keymap::Action::AiToggleThought(1))
            .unwrap();
        let opened = row_texts(&build_chat_lines(&app, 60, 30, ChatPlacement::Tab)).join("\n");
        assert!(
            opened.contains("SECOND_THOUGHT") && opened.contains("FINAL_ANSWER"),
            "{opened}"
        );
    }

    #[test]
    fn live_reasoning_takes_one_row_and_wait_chrome_is_not_copyable_content() {
        let (mut app, _project, _store) = activity_test_app();
        for width in [18, 32, 80] {
            let mut rows = Vec::new();
            push_thinking_state(
                &mut rows,
                "中文推理\nA second line with more detail",
                width,
                false,
                true,
            );
            assert_eq!(rows.len(), 1, "{width}: {rows:?}");
            assert!(rows[0].width() <= width);
        }
        app.agents[0].status = AiStatus::Waiting;
        app.agents[0].chat.last_activity =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        let projection = build_chat_projection(&app, 80, 30, ChatPlacement::Tab);
        assert!(
            projection
                .lines
                .iter()
                .any(|row| line_text(&row.line).contains("Waiting for agent"))
        );
        assert!(
            projection
                .blocks
                .iter()
                .all(|block| !block.text.contains("Waiting for agent"))
        );
        app.agents[0]
            .partial_response_mut()
            .push_str("REPLY_STREAM");
        let rows = row_texts(&build_chat_lines(&app, 80, 30, ChatPlacement::Tab)).join("\n");
        assert!(
            rows.contains("REPLY_STREAM") && !rows.contains("Waiting for agent"),
            "{rows}"
        );
    }

    #[test]
    fn unfinished_or_cancelled_processes_are_not_automatically_folded() {
        let (mut app, _project, _store) = activity_test_app();
        for state in [
            AiStatus::Waiting,
            AiStatus::AwaitingApproval,
            AiStatus::Error,
        ] {
            *app.agents[0].messages_mut() = vec![
                message(AiRole::User, "Question", None),
                message(AiRole::Assistant, "PROGRESS_DETAIL", Some("agent_progress")),
                message(AiRole::Assistant, "incomplete", Some("agent_progress")),
            ];
            app.agents[0].status = state;
            let rows = row_texts(&build_chat_lines(&app, 80, 30, ChatPlacement::Tab)).join("\n");
            assert!(rows.contains("PROGRESS_DETAIL"), "{state:?}: {rows}");
        }
    }

    #[test]
    fn activity_layout_reuses_completed_groups_and_tracks_updates() {
        let (mut app, _project, _store) = activity_test_app();
        for index in 0..1000 {
            app.agents[0].messages_mut().push(AiChatMessage {
                role: AiRole::Assistant,
                content: format!("Read file {index}: 中文 tool title"),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: Some("completed".into()),
                source: Some("acp_activity".into()),
                activity: None,
            });
        }
        let started = std::time::Instant::now();
        let first = build_chat_projection(&app, 80, 24, ChatPlacement::Tab);
        let cold = started.elapsed();
        let started = std::time::Instant::now();
        let second = build_chat_projection(&app, 80, 24, ChatPlacement::Tab);
        eprintln!("1000 tools: cold={cold:?}, warm={:?}", started.elapsed());
        assert!(std::rc::Rc::ptr_eq(&first.lines[0], &second.lines[0]));
        app.agents[0].messages_mut()[500].reasoning_content = Some("failed".into());
        let changed = build_chat_projection(&app, 80, 24, ChatPlacement::Tab);
        assert!(!std::rc::Rc::ptr_eq(&second.lines[0], &changed.lines[0]));
        assert!(
            changed
                .lines
                .iter()
                .any(|line| line_text(&line.line).contains("1 failed"))
        );
        app.agents[0].messages_mut().clear();
        build_chat_projection(&app, 80, 24, ChatPlacement::Tab);
        assert!(app.chat_layout_cache.borrow().activities.is_empty());
    }

    #[test]
    fn history_layout_reuses_rows_and_invalidates_content_and_width() {
        let (mut app, _project, _store) = activity_test_app();
        for index in 0..1000 {
            app.agents[0].messages_mut().push(AiChatMessage {
                role: AiRole::Assistant,
                content: format!("Message {index}: **中文** repeated text\n").repeat(8),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: None,
                activity: None,
            });
        }
        let started = std::time::Instant::now();
        let first = build_chat_projection(&app, 80, 24, ChatPlacement::Tab);
        let cold = started.elapsed();
        let started = std::time::Instant::now();
        let second = build_chat_projection(&app, 80, 24, ChatPlacement::Tab);
        eprintln!("1000 messages: cold={cold:?}, warm={:?}", started.elapsed());
        assert_eq!(first.lines.len(), second.lines.len());
        assert!(std::rc::Rc::ptr_eq(&first.lines[0], &second.lines[0]));
        app.agents[0].messages_mut()[0].content = "Changed".into();
        let changed = build_chat_projection(&app, 80, 24, ChatPlacement::Tab);
        assert!(!std::rc::Rc::ptr_eq(&second.lines[0], &changed.lines[0]));
        let resized = build_chat_projection(&app, 40, 24, ChatPlacement::Tab);
        assert!(!std::rc::Rc::ptr_eq(&changed.lines[0], &resized.lines[0]));
        app.agents[0].messages_mut().clear();
        build_chat_projection(&app, 40, 24, ChatPlacement::Tab);
        assert!(app.chat_layout_cache.borrow().entries.is_empty());
    }

    #[test]
    fn progress_header_is_muted_and_final_header_is_green() {
        let progress = role_header(AiRole::Assistant, Some("agent_progress"), false);
        assert_eq!(progress.spans.len(), 1);
        assert_eq!(progress.spans[0].style.fg, Some(theme::MUTED));
        let final_reply = role_header(AiRole::Assistant, None, false);
        assert_eq!(final_reply.spans[0].style.fg, Some(theme::SUCCESS));
        assert_eq!(
            final_reply.spans.len(),
            1,
            "unfinished replies carry no mark"
        );
    }

    #[test]
    fn every_finished_answer_keeps_its_green_rail() {
        let (mut app, _project, _store) = activity_test_app();
        app.agents[0].messages_mut().extend([
            message(AiRole::User, "QUESTION_ONE", None),
            message(AiRole::Assistant, "ANSWER_ONE", None),
            message(AiRole::User, "QUESTION_TWO", None),
            message(AiRole::Assistant, "ANSWER_TWO", None),
        ]);
        // The cue sits on the reply's header, so the marked row is the one that opens the
        // answer; the row under it is what the user reads.
        let marks = |app: &App| {
            let projection = build_chat_projection(app, 80, 40, ChatPlacement::Tab);
            let rows: Vec<String> = projection
                .lines
                .iter()
                .map(|row| line_text(&row.line))
                .collect();
            rows.iter()
                .enumerate()
                .filter_map(|(index, text)| {
                    if !text.contains('✓') {
                        return None;
                    }
                    let mark = projection.lines[index]
                        .line
                        .spans
                        .iter()
                        .find(|span| span.content.contains('✓'))
                        .unwrap();
                    assert_eq!(mark.style.fg, Some(theme::SUCCESS), "completion mark");
                    let rail = &projection.lines[index].line.spans[0];
                    assert_eq!(rail.content, RAIL, "finished answers keep the green bar");
                    assert_eq!(rail.style.fg, Some(theme::SUCCESS), "green bar colour");
                    rows[index + 1..]
                        .iter()
                        .find(|row| !row.is_empty())
                        .cloned()
                })
                .collect::<Vec<_>>()
        };
        app.agents[0].completed_answers = [1, 3].into_iter().collect();
        let marked = marks(&app);
        assert_eq!(
            marked.len(),
            2,
            "each finished answer is marked: {marked:?}"
        );
        assert!(marked[0].contains("ANSWER_ONE"), "{marked:?}");
        assert!(marked[1].contains("ANSWER_TWO"), "{marked:?}");

        app.agents[0].status = AiStatus::Waiting;
        let running = marks(&app);
        assert_eq!(
            running.len(),
            2,
            "finished answers keep their cues during the next turn: {running:?}"
        );

        app.agents[0].messages_mut().extend([
            message(AiRole::User, "QUESTION_THREE", None),
            message(AiRole::Assistant, "ANSWER_THREE", None),
        ]);
        app.agents[0].completed_answers.insert(5);
        app.agents[0].status = AiStatus::Idle;
        let newest = marks(&app);
        assert_eq!(newest.len(), 3, "{newest:?}");
        assert!(newest[2].contains("ANSWER_THREE"), "{newest:?}");

        app.agents[0].messages_mut().extend([
            message(AiRole::User, "QUESTION_FOUR", None),
            message(AiRole::Assistant, "CANCELLED_ANSWER", None),
        ]);
        app.agents[0].last_finish_reason = Some("cancelled".into());
        let cancelled = marks(&app);
        assert_eq!(cancelled.len(), 3, "{cancelled:?}");
        assert!(
            !cancelled.iter().any(|row| row.contains("CANCELLED_ANSWER")),
            "a cancelled turn earns no marker: {cancelled:?}"
        );
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
    fn metrics_row_keeps_elapsed_and_context_outside_the_input_border() {
        use ratatui::{Terminal, backend::TestBackend};
        let (mut app, _project, _store) = activity_test_app();
        app.agents[0].task_elapsed = std::time::Duration::from_secs(125);
        app.agents[0].task_started = None;
        app.agents[0].last_input_tokens = None;
        let mut terminal = Terminal::new(TestBackend::new(100, 12)).unwrap();
        for focused in [true, false] {
            terminal
                .draw(|frame| {
                    render_chat_panel(frame, &mut app, frame.area(), focused, ChatPlacement::Tab)
                })
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
    fn completed_marker_survives_the_next_running_turn() {
        let (mut app, _project, _store) = activity_test_app();
        app.active_tab = crate::app::MainTab::Ai;
        app.agents[0]
            .messages_mut()
            .push(message(AiRole::Assistant, "first answer", None));
        app.agents[0].completed_answers.insert(0);
        app.agents[0].status = AiStatus::Waiting;
        app.agents[0]
            .partial_response_mut()
            .push_str("second answer in progress");
        let projection = build_chat_projection(&app, 80, 40, ChatPlacement::Tab);
        let rows: Vec<String> = projection
            .lines
            .iter()
            .map(|row| line_text(&row.line))
            .collect();
        assert!(
            rows.iter().any(|row| row.contains('✓')),
            "the previous answer keeps its completion cue: {rows:?}"
        );
    }

    #[test]
    fn finished_turn_leaves_no_stream_cursor() {
        let (mut idle, _project, _store) = activity_test_app();
        idle.active_tab = crate::app::MainTab::Ai;
        idle.agents[0]
            .partial_response_mut()
            .push_str("partial text");
        idle.agents[0].status = AiStatus::Idle;
        let idle_projection = build_chat_projection(&idle, 80, 40, ChatPlacement::Tab);
        assert!(
            idle_projection
                .lines
                .iter()
                .all(|row| !line_text(&row.line).contains(STREAM_CURSOR)),
            "a finished turn must not keep the streaming cursor"
        );

        let (mut running, _project, _store) = activity_test_app();
        running.active_tab = crate::app::MainTab::Ai;
        running.agents[0]
            .partial_response_mut()
            .push_str("partial text");
        running.agents[0].status = AiStatus::Waiting;
        let running_projection = build_chat_projection(&running, 80, 40, ChatPlacement::Tab);
        assert!(
            running_projection
                .lines
                .iter()
                .any(|row| line_text(&row.line).contains(STREAM_CURSOR)),
            "a running turn keeps the streaming cursor"
        );
    }

    #[test]
    fn chat_code_blocks_register_independent_copy_hitboxes() {
        use ratatui::{Terminal, backend::TestBackend};
        let (mut app, _project, _store) = activity_test_app();
        app.active_tab = crate::app::MainTab::Ai;
        let text = "~~~rust\nfn main() {}\n~~~\n\n~~~sh\necho hi\n~~~";
        app.agents[0]
            .messages_mut()
            .push(message(AiRole::Assistant, text, None));
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        terminal
            .draw(|frame| {
                render_chat_panel(frame, &mut app, frame.area(), true, ChatPlacement::Tab)
            })
            .unwrap();
        let mut hits = Vec::new();
        for region in &app.clickable_regions {
            if let ClickableRegion::ActionButton {
                rect,
                action:
                    crate::keymap::Action::AiCopyCodeBlock {
                        message_index,
                        block_index,
                    },
                ..
            } = region
            {
                hits.push((*message_index, *block_index, *rect));
            }
        }
        assert_eq!(hits.len(), 2, "one hitbox per fenced block");
        assert_eq!((hits[0].0, hits[0].1), (0, 0));
        assert_eq!((hits[1].0, hits[1].1), (0, 1));
        let label_width = crate::markdown::CODE_COPY_LABEL.width() as u16;
        assert!(hits.iter().all(|hit| hit.2.width == label_width));
        assert_ne!(hits[0].2.y, hits[1].2.y);
        let history = app.chat_history_rect.expect("chat renders");
        for region in &app.clickable_regions {
            if let ClickableRegion::ActionButton { rect, action, .. } = region
                && history.contains(ratatui::layout::Position::new(rect.x, rect.y))
            {
                assert!(
                    crate::app::is_chat_inline_control(action),
                    "a chat row hitbox must run its action instead of starting a selection: {action:?}"
                );
            }
        }
        let label = hits[0].2;
        let before = terminal.backend().buffer()[(label.x, label.y)].fg;
        assert_eq!(before, theme::MUTED);
        app.handle_mouse_event(
            crossterm::event::MouseEventKind::Moved,
            label.x + 1,
            label.y,
            crossterm::event::KeyModifiers::NONE,
        )
        .unwrap();
        terminal
            .draw(|frame| {
                render_chat_panel(frame, &mut app, frame.area(), true, ChatPlacement::Tab)
            })
            .unwrap();
        assert_eq!(
            terminal.backend().buffer()[(label.x, label.y)].fg,
            theme::PRIMARY,
            "hovering the copy label highlights it"
        );
    }

    #[test]
    fn chat_scrollbar_paints_one_solid_cell_and_highlights_on_hover() {
        use ratatui::{Terminal, backend::TestBackend};
        let (mut app, _project, _store) = activity_test_app();
        app.active_tab = crate::app::MainTab::Ai;
        for index in 0..40 {
            app.agents[0].messages_mut().push(message(
                AiRole::Assistant,
                &format!("scrollable line {index}"),
                None,
            ));
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        terminal
            .draw(|frame| {
                render_chat_panel(frame, &mut app, frame.area(), true, ChatPlacement::Tab)
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width;
        let thumb: Vec<(u16, u16)> = buffer
            .content()
            .iter()
            .enumerate()
            .filter(|(_, cell)| cell.bg == theme::SCROLL_THUMB)
            .map(|(index, _)| ((index as u16) % width, (index as u16) / width))
            .collect();
        let (thumb_x, thumb_y) = *thumb.first().expect("a scrollbar thumb");
        assert!(
            !thumb
                .iter()
                .any(|(x, y)| *x == thumb_x + 1 && *y == thumb_y),
            "the thumb is one cell wide"
        );

        app.handle_mouse_event(
            crossterm::event::MouseEventKind::Moved,
            thumb_x,
            thumb_y,
            crossterm::event::KeyModifiers::NONE,
        )
        .unwrap();
        terminal
            .draw(|frame| {
                render_chat_panel(frame, &mut app, frame.area(), true, ChatPlacement::Tab)
            })
            .unwrap();
        assert_eq!(
            terminal.backend().buffer()[(thumb_x, thumb_y)].bg,
            theme::SCROLL_THUMB_HOVER,
            "hovering the bar lifts its thumb"
        );
    }

    #[test]
    fn chat_scrollbar_pages_on_track_click_and_drags_the_thumb() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
        use ratatui::{Terminal, backend::TestBackend};
        let (mut app, _project, _store) = activity_test_app();
        app.active_tab = crate::app::MainTab::Ai;
        for index in 0..40 {
            app.agents[0].messages_mut().push(message(
                AiRole::Assistant,
                &format!("scrollable line {index}"),
                None,
            ));
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        let mut draw = |app: &mut crate::app::App| {
            terminal
                .draw(|frame| render_chat_panel(frame, app, frame.area(), true, ChatPlacement::Tab))
                .unwrap();
        };
        draw(&mut app);
        let bar = app.chat_scrollbar.expect("overflowing history draws a bar");
        let height = bar.rect.height as usize;
        let max_start = bar.total - height;
        let x = bar.rect.x;
        assert_eq!(app.agent().scroll_offset, 0);

        // Clicking the track above the thumb pages up by one viewport.
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            x,
            bar.rect.y,
            KeyModifiers::NONE,
        )
        .unwrap();
        app.handle_mouse_event(
            MouseEventKind::Up(MouseButton::Left),
            x,
            bar.rect.y,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert_eq!(app.agent().scroll_offset, height);

        // Grabbing the thumb moves the transcript with the pointer.
        draw(&mut app);
        let start = max_start - app.agent().scroll_offset;
        let (thumb_pos, _) = crate::theme::scrollbar_thumb(
            bar.total,
            height,
            start,
            crate::theme::ScrollAxis::Vertical,
        )
        .unwrap();
        let grab_y = bar.rect.y + thumb_pos as u16;
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            x,
            grab_y,
            KeyModifiers::NONE,
        )
        .unwrap();
        app.handle_mouse_event(
            MouseEventKind::Drag(MouseButton::Left),
            x,
            bar.rect.y,
            KeyModifiers::NONE,
        )
        .unwrap();
        app.handle_mouse_event(
            MouseEventKind::Up(MouseButton::Left),
            x,
            bar.rect.y,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert_eq!(
            app.agent().scroll_offset,
            max_start,
            "dragging the thumb to the top shows the first row"
        );
        assert!(app.chat_scrollbar_grab.is_none());
    }

    #[test]
    fn composer_buttons_queue_complete_unicode_drafts_without_covering_the_input() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
        use ratatui::{Terminal, backend::TestBackend};
        for width in [20, 32, 48, 90, 120] {
            let (mut app, _project, _store) = activity_test_app();
            app.active_tab = crate::app::MainTab::Ai;
            app.ai_input_focused = true;
            app.agents[0].status = AiStatus::Waiting;
            let draft = "中文导航 first line\nsecond line with a long wrapped follow-up";
            app.agents[0].input = draft.into();
            app.agents[0].input_cursor = draft.chars().count();
            let mut terminal = Terminal::new(TestBackend::new(width, 22)).unwrap();
            terminal
                .draw(|frame| {
                    render_chat_panel(frame, &mut app, frame.area(), true, ChatPlacement::Tab)
                })
                .unwrap();
            let input = app
                .clickable_regions
                .iter()
                .find_map(|region| match region {
                    ClickableRegion::ChatInput { rect } => Some(*rect),
                    _ => None,
                })
                .unwrap();
            let button = app
                .clickable_regions
                .iter()
                .find_map(|region| match region {
                    ClickableRegion::ActionButton {
                        rect,
                        action: crate::keymap::Action::AiSendMessage,
                        ..
                    } => Some(*rect),
                    _ => None,
                });
            let cursor = terminal.get_cursor_position().unwrap();
            assert!(
                cursor.x < width - 1 && cursor.y < input.bottom() - 1,
                "cursor stays inside the composer after Unicode wrapping: {cursor:?}"
            );
            if let Some(button) = button {
                assert!(
                    cursor.x < button.x,
                    "cursor must not land in the send control"
                );
                assert!(
                    button.x >= input.right(),
                    "button must not cover editable cells at width {width}"
                );
                assert!(button.right() < width && button.y < 22);
                app.handle_mouse_event(
                    MouseEventKind::Down(MouseButton::Left),
                    button.x + 1,
                    button.y,
                    KeyModifiers::NONE,
                )
                .unwrap();
            } else {
                assert!(width < 30, "normal-width composers retain the send control");
                app.handle_action(crate::keymap::Action::AiSendMessage)
                    .unwrap();
            }
            assert_eq!(
                app.agent().chat.queue.front().map(String::as_str),
                Some(draft)
            );
            assert!(
                app.agent().messages().is_empty(),
                "queued text is not an already submitted turn"
            );
        }
    }

    #[test]
    fn tinted_user_blocks_and_green_answer_rails_preserve_selection_across_wraps() {
        let (mut app, _project, _store) = activity_test_app();
        app.agents[0].messages_mut().extend([
            message(
                AiRole::User,
                "中文导航 first **bold** line\nsecond line",
                None,
            ),
            message(AiRole::Assistant, "ANSWER_BODY", None),
        ]);
        app.agents[0].completed_answers.insert(1);
        for width in [24, 48, 80] {
            let projection = build_chat_projection(&app, width, 40, ChatPlacement::Tab);
            let rows: Vec<_> = projection
                .lines
                .iter()
                .enumerate()
                .filter(|(_, row)| row.source.as_ref().is_some_and(|source| source.block == 0))
                .collect();
            assert!(!rows.is_empty());
            for (_, row) in &rows {
                assert_eq!(row.line.spans[0].content, RAIL);
                assert!(
                    row.line
                        .spans
                        .iter()
                        .skip(1)
                        .all(|span| span.style.bg == Some(theme::CHAT_USER_BG))
                );
            }
            let (first, row) = rows.first().unwrap();
            let start = projection
                .anchor_at(*first, row.source.as_ref().unwrap().display_start)
                .unwrap();
            let (last, row) = rows.last().unwrap();
            let end = projection
                .anchor_at(*last, row.source.as_ref().unwrap().display_end)
                .unwrap();
            let selection = ChatSelection {
                agent_id: 0,
                anchor: start,
                focus: end,
            };
            assert_eq!(
                projection.selected_text(selection).as_deref(),
                Some("中文导航 first bold line\nsecond line")
            );
            let answer = projection
                .lines
                .iter()
                .find(|row| line_text(&row.line).contains("ANSWER_BODY"))
                .unwrap();
            assert_eq!(answer.line.spans[0].content, RAIL, "finished answer bar");
            assert_eq!(
                answer.line.spans[0].style.fg,
                Some(theme::SUCCESS),
                "finished answer bar colour"
            );
        }
    }

    #[test]
    fn metrics_keep_warning_fields_whole_and_remain_above_the_composer() {
        use ratatui::{Terminal, backend::TestBackend};
        let (mut app, _project, _store) = activity_test_app();
        app.agents[0].last_input_tokens = Some(app.active_context_window());
        app.agents[0].status = AiStatus::Waiting;
        app.agents[0]
            .chat
            .rate
            .record("live output", std::time::Instant::now());
        for width in [12, 18, 24, 40, 80] {
            let label = chat_metrics(&app, width);
            assert!(label.width() <= width);
            assert!(
                label.ends_with('!'),
                "context warning survives compact layouts: {label}"
            );
            assert!(
                !label.contains('…'),
                "metric fields never get clipped mid-number"
            );
        }
        let mut terminal = Terminal::new(TestBackend::new(100, 16)).unwrap();
        terminal
            .draw(|frame| {
                render_chat_panel(frame, &mut app, frame.area(), true, ChatPlacement::Tab)
            })
            .unwrap();
        let input = app
            .clickable_regions
            .iter()
            .find_map(|region| match region {
                ClickableRegion::ChatInput { rect } => Some(*rect),
                _ => None,
            })
            .unwrap();
        let line = |y| {
            (0..100)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect::<String>()
        };
        assert!(line(input.y - 1).contains("ctx"));
        assert!(!line(input.y).contains("ctx"));
    }

    #[test]
    fn test_focused_composer_paints_block_cursor_at_caret() {
        use ratatui::{Terminal, backend::TestBackend};

        fn cursor_cells(
            terminal: &mut Terminal<TestBackend>,
            app: &mut App,
        ) -> Vec<((u16, u16), String)> {
            terminal
                .draw(|frame| render_chat_panel(frame, app, frame.area(), true, ChatPlacement::Tab))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let area = buffer.area;
            (area.top()..area.bottom())
                .flat_map(|y| (area.left()..area.right()).map(move |x| (x, y)))
                .filter(|&position| buffer[position].bg == theme::CURSOR_BG)
                .map(|position| (position, buffer[position].symbol().to_string()))
                .collect()
        }

        let (mut app, _project, _store) = activity_test_app();
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();

        // Cases: (draft, caret, expected glyphs under the block).
        let cases: [(&str, usize, &[&str]); 4] = [
            ("", 0, &["T"]),
            ("ab", 1, &["b"]),
            ("ab", 2, &[" "]),
            ("a中", 1, &["中", " "]),
        ];
        for (draft, caret, expected) in cases {
            app.ai_input_focused = true;
            app.agent_mut().input = draft.to_string();
            app.agent_mut().input_cursor = caret;
            let cells = cursor_cells(&mut terminal, &mut app);
            let glyphs: Vec<&str> = cells.iter().map(|(_, glyph)| glyph.as_str()).collect();
            assert_eq!(glyphs, expected, "block cursor for {draft:?} at {caret}");
            assert_eq!(
                terminal.get_cursor_position().unwrap(),
                cells[0].0.into(),
                "the hardware cursor stays on the block for IME anchoring"
            );
        }

        // A fresh backend: diff rendering skips wide-glyph continuation cells,
        // so a reused test buffer would keep the previous frame's block.
        app.ai_input_focused = false;
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        let cells = cursor_cells(&mut terminal, &mut app);
        assert!(
            cells.is_empty(),
            "an unfocused composer shows no caret: {cells:?}"
        );
    }

    #[test]
    fn context_warning_uses_last_input_and_seventy_percent_boundary() {
        use ratatui::{Terminal, backend::TestBackend};
        let (mut app, _project, _store) = activity_test_app();
        let limit = app.active_context_window();
        let threshold = (u64::from(limit) * 7).div_ceil(10) as u32;
        let mut terminal = Terminal::new(TestBackend::new(100, 12)).unwrap();
        for (tokens, warning) in [(threshold - 1, false), (threshold, true)] {
            app.agents[0].last_input_tokens = Some(tokens);
            app.agents[0].total_input_tokens = 0;
            terminal
                .draw(|frame| {
                    render_chat_panel(frame, &mut app, frame.area(), true, ChatPlacement::Tab)
                })
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
            activity: None,
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
                activity: None,
            },
            AiChatMessage {
                role: AiRole::Assistant,
                content: "third **answer**".into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: None,
                activity: None,
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
                activity: None,
            },
            AiChatMessage {
                role: AiRole::Assistant,
                content: "WRAPPED_REPLY_01".into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: None,
                activity: None,
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
        // A finished answer switches to the green bar; anything else keeps the quiet guide.
        let (glyph, rail_color) = if expected == theme::SUCCESS {
            (RAIL, theme::SUCCESS)
        } else {
            (QUIET_RAIL, theme::BORDER)
        };
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
        assert_eq!(rows[index].spans[0].content, glyph, "reply rail glyph");
        assert_eq!(
            rows[index].spans[0].style.fg,
            Some(rail_color),
            "reply rail colour"
        );
        let header = (0..index)
            .rev()
            .find(|i| texts[*i].contains("Teshi"))
            .unwrap();
        assert_eq!(rows[header].spans[0].content, glyph, "header rail glyph");
        assert_eq!(
            rows[header].spans[0].style.fg,
            Some(rail_color),
            "header rail colour"
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
            rail_color,
            "rendered reply rail"
        );
        assert_eq!(
            buffer[(0, header as u16)].fg,
            rail_color,
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
                    .position(|row| row.contains("Done Read project"))
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
        push_railed(&mut out, vec![long], RailKind::User, 12);
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
            activity: None,
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
            activity: None,
            role: AiRole::Assistant,
            content: "Answer".into(),
            reasoning_content: Some("Inspecting the project".into()),
            source: None,
            tool_calls: None,
            tool_call_id: None,
        };
        let tool = AiChatMessage {
            activity: None,
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
            "Done Read project",
            "Live reasoning before completion",
        ] {
            assert!(screen.contains(expected), "Missing {expected}: {screen}");
        }
    }

    #[test]
    fn a_turn_keeps_one_guide_column_and_ends_on_the_green_answer_bar() {
        let (mut app, _project, _store) = activity_test_app();
        let mut thought = message(AiRole::Assistant, "", Some("acp_reasoning"));
        thought.reasoning_content = Some("FIRST_THOUGHT\nSECOND_THOUGHT".into());
        let mut call = message(AiRole::Assistant, "Read project", Some("acp_activity"));
        call.reasoning_content = Some("completed".into());
        call.tool_call_id = Some("read-1".into());
        app.agents[0].activity_details_expanded = true;
        app.agents[0].messages_mut().extend([
            message(AiRole::User, "QUESTION", None),
            thought,
            call,
            message(AiRole::Assistant, "ANSWER", None),
        ]);
        app.agents[0].completed_answers.insert(3);

        let lines = build_chat_lines(&app, 72, 40, ChatPlacement::Tab);
        let rows = row_texts(&lines);
        let answer = rows
            .iter()
            .position(|row| row.contains("ANSWER"))
            .expect("finished answer row");
        assert_eq!(
            lines[answer].spans[0].content, RAIL,
            "the finished answer opens the green bar: {rows:?}"
        );
        assert_eq!(
            lines[answer].spans[0].style.fg,
            Some(theme::SUCCESS),
            "the finished answer bar is green"
        );
        let first = rows
            .iter()
            .position(|row| row.starts_with(QUIET_RAIL))
            .unwrap();
        let last = rows[..answer]
            .iter()
            .rposition(|row| row.starts_with(QUIET_RAIL))
            .unwrap();
        assert!(
            first > 0 && rows[first - 1].is_empty(),
            "a turn opens after an empty gap: {rows:?}"
        );
        assert!(
            rows[first..=last].iter().all(|row| !row.is_empty()),
            "the guide breaks inside a turn: {rows:?}"
        );
        assert!(
            rows[first..=last]
                .iter()
                .all(|row| row.starts_with(QUIET_RAIL)),
            "every activity row of a turn sits on the guide: {rows:?}"
        );
    }

    #[test]
    fn tool_results_sit_on_the_guide() {
        let mut result = message(AiRole::Tool, "first line\nsecond line", None);
        result.tool_call_id = Some("read-1".into());
        let mut out = Vec::new();
        push_tool_result(&mut out, &result, 40);
        assert_eq!(out.len(), 3);
        assert!(
            row_texts(&out)
                .iter()
                .all(|row| row.starts_with(QUIET_RAIL)),
            "tool results indent from the guide: {:?}",
            row_texts(&out)
        );
    }

    #[test]
    fn compact_activity_keeps_status_and_expands_full_unicode_title() {
        let title = "Read 项目目录 with a very long command and all its arguments";
        let tool = AiChatMessage {
            activity: None,
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
        assert_eq!(thoughts.len(), 1);
        assert!(
            thoughts
                .iter()
                .flat_map(|line| line.spans.iter().skip(1))
                .all(|span| span.style.fg == Some(theme::MUTED))
        );
        assert_eq!(thoughts[0].spans[0].style.fg, Some(theme::BORDER));
        thoughts.clear();
        push_thinking(&mut thoughts, "First line\nSecond line", 72, true);
        assert_eq!(thoughts.len(), 3);
    }

    #[test]
    fn test_image_box_keeps_aspect_within_width_and_row_cap() {
        let font = (10, 20);
        let cases = [
            // A 16:9 screenshot reaches the row cap before the body width.
            ((1920, 1080), 100, (57, 16)),
            // A narrow body caps the width instead.
            ((1920, 1080), 40, (40, 12)),
            // A tall image is capped by rows, not stretched.
            ((400, 2000), 100, (7, 16)),
            // A small image keeps its native size.
            ((50, 40), 100, (5, 2)),
            ((0, 40), 100, (0, 0)),
        ];
        for (pixels, max_cols, expected) in cases {
            assert_eq!(
                image_box(pixels, font, max_cols),
                expected,
                "{pixels:?} in {max_cols} cols"
            );
        }
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
