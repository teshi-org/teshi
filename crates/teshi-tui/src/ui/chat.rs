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
    AiChatMessage, AiRole, AiStatus, App, ChangeKind, ClickableRegion, SLASH_COMMANDS,
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
pub(super) fn render_agent_sidebar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.width < 5 || area.height < 3 {
        return;
    }
    let block = theme::panel_block(" Agents ", false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, agent) in app.agents.iter().enumerate() {
        let (dot, dot_color) = match agent.status {
            AiStatus::Waiting => ("●", theme::WARNING),
            AiStatus::AwaitingApproval => ("◆", theme::PRIMARY),
            AiStatus::Error => ("●", theme::ERROR),
            AiStatus::Idle => ("○", theme::SUCCESS),
        };
        let title = truncate_chars(&agent.title, 12);
        let line = if i == app.selected_agent {
            Line::from(format!(" {dot} {title}")).style(selected_row())
        } else {
            Line::from(vec![
                Span::raw(" "),
                Span::styled(dot, fg(dot_color)),
                Span::styled(format!(" {title}"), fg(theme::FG)),
            ])
        };
        lines.extend(wrap_line_to_width(line, inner.width as usize));
    }

    lines.push(Line::default());
    lines.push(Line::styled(" a new · x close", muted()));
    lines.push(Line::styled(" j/k switch", muted()));

    let visible: Vec<Line<'static>> = lines.into_iter().take(inner.height as usize).collect();
    frame.render_widget(Paragraph::new(Text::from(visible)), inner);
}

/// Truncates to `max` characters, marking the cut with an ellipsis.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
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

    let title = Line::from(vec![
        Span::styled(" AI Chat ", fg(theme::PRIMARY).add_modifier(Modifier::BOLD)),
        Span::styled(format!("· {} ", app.active_agent_profile().name), muted()),
    ]);
    let block = theme::panel_block(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_approval_badge(frame, app, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let input_active = focused && app.ai_input_focused;
    let status_height: u16 = 1;

    // The input grows with its wrapped content but never takes more than a
    // third of the panel, so history always stays visible.
    let input_text_width = inner.width.saturating_sub(2);
    let input_text_rows = visual_line_count_for_width(&app.agent().input, input_text_width)
        .min((inner.height / 3).max(3));
    let input_height: u16 = input_text_rows + 2;

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
    render_chat_status(frame, app, status_area);
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
    let mode = &app.approval_mode;
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
fn render_chat_history(frame: &mut Frame<'_>, app: &App, area: Rect, placement: ChatPlacement) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let [body, scrollbar] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).areas(area);
    let lines = build_chat_lines(app, body.width as usize, body.height as usize, placement);

    // Lines are already wrapped, so the row count is exact. `scroll_offset`
    // counts rows up from the bottom so new output stays pinned by default.
    let total = lines.len();
    let height = body.height as usize;
    let max_start = total.saturating_sub(height);
    let start = max_start.saturating_sub(app.agent().scroll_offset.min(max_start));
    let visible: Vec<Line<'static>> = lines.into_iter().skip(start).take(height).collect();
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
fn build_chat_lines(
    app: &App,
    width: usize,
    view_height: usize,
    placement: ChatPlacement,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let agent = app.agent();

    if agent.messages().is_empty() {
        let greeting = match placement {
            ChatPlacement::Tab => "Welcome to AI Chat! Type a message below and press Enter.",
            ChatPlacement::Embedded => "Welcome to AI Chat!",
        };
        out.extend(wrap_line_to_width(Line::styled(greeting, muted()), width));
        out.push(Line::default());
        if !crate::llm::is_configured() {
            let note =
                "Note: Run 'teshi auth login' or set TESHI_LLM_API_KEY to enable AI responses.";
            out.extend(wrap_line_to_width(
                Line::styled(note, fg(theme::WARNING)),
                width,
            ));
        }
    }

    for msg in agent.messages() {
        match msg.role {
            AiRole::Tool => push_tool_result(&mut out, msg, width),
            AiRole::User | AiRole::Assistant => push_message(&mut out, app, msg, width),
        }
        out.push(Line::default());
    }

    push_pending_diff(&mut out, app, width, view_height);

    if !agent.partial_response().is_empty() {
        let mut body = vec![role_header(AiRole::Assistant, None)];
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
        push_railed(&mut out, body, theme::SUCCESS, width);
        out.push(Line::default());
    }

    push_acp_permission_prompt(&mut out, app, width);
    out
}

fn rail_body_width(width: usize) -> u16 {
    width.saturating_sub(RAIL.width()).min(u16::MAX as usize) as u16
}

fn role_header(role: AiRole, source: Option<&str>) -> Line<'static> {
    let (label, color) = match role {
        AiRole::User => ("You", theme::ACCENT),
        _ => ("Teshi", theme::SUCCESS),
    };
    let mut spans = Vec::with_capacity(2);
    if let Some(source) = source {
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
    let mut body = vec![role_header(msg.role, msg.source.as_deref())];
    if !msg.content.is_empty() {
        body.extend(render_markdown_width(&msg.content, rail_body_width(width)));
    }
    if let Some(tool_calls) = &msg.tool_calls {
        body.extend(tool_calls.iter().map(|tc| tool_call_line(app, tc)));
    }
    let color = match msg.role {
        AiRole::User => theme::ACCENT,
        _ => theme::SUCCESS,
    };
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
fn render_chat_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let agent = app.agent();
    let thinking = agent.status == AiStatus::Waiting && agent.partial_response().is_empty();
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

    let model_label = app.active_model_label.as_deref().unwrap_or("");
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

fn render_slash_suggestions(frame: &mut Frame<'_>, app: &App, area: Rect, filter: &str) {
    let filtered = filtered_slash_commands(filter);
    let selection = app
        .slash_suggestion_selection
        .min(filtered.len().saturating_sub(1));
    let width = area.width as usize;

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(filtered.len() + 1);
    for (i, (name, desc)) in filtered.iter().enumerate() {
        let line = if i == selection {
            Line::from(format!(" /{name}  {desc}")).style(selected_row())
        } else {
            Line::from(vec![
                Span::styled(format!(" /{name}"), fg(theme::PRIMARY)),
                Span::styled(format!("  {desc}"), muted()),
            ])
        };
        lines.extend(wrap_line_to_width(line, width).into_iter().take(1));
    }
    lines.push(Line::styled(
        " ↑↓ navigate · Enter/Tab select · Esc close",
        muted(),
    ));

    let visible: Vec<Line<'static>> = lines.into_iter().take(area.height as usize).collect();
    frame.render_widget(Paragraph::new(Text::from(visible)), area);
}

fn render_chat_input(frame: &mut Frame<'_>, app: &App, area: Rect, active: bool) {
    let border_color = if active { theme::ACCENT } else { theme::BORDER };
    let agent = app.agent();
    let label = token_usage_label(agent.total_input_tokens, agent.total_output_tokens);
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

    let display: Text<'static> = if agent.input.is_empty() {
        if active {
            Text::from(Line::styled("Type your message...", muted()))
        } else {
            Text::default()
        }
    } else {
        // Build lines explicitly so Paragraph renders multi-line input correctly.
        Text::from(
            agent
                .input
                .lines()
                .map(|l| Line::raw(l.to_string()))
                .collect::<Vec<_>>(),
        )
    };
    let text_style = match agent.status {
        AiStatus::Waiting => muted(),
        _ => Style::default(),
    };
    frame.render_widget(
        Paragraph::new(display)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .style(text_style),
        inner,
    );

    // Show a visible cursor at the insertion point, accounting for wrapping.
    if active && agent.status != AiStatus::Waiting {
        let (row, col) = visual_cursor_pos(&agent.input, agent.input_cursor, inner.width);
        frame.set_cursor_position((inner.x + col, inner.y + row));
    }
}

/// Number of visual (wrapped) rows the input text occupies when rendered in a
/// widget of the given `width` (in columns).
fn visual_line_count_for_width(input: &str, width: u16) -> u16 {
    if width == 0 {
        return input.lines().count().max(1) as u16;
    }
    let count: u16 = input
        .lines()
        .map(|line| {
            let w: u16 = line.chars().map(|c| c.width().unwrap_or(0) as u16).sum();
            if w == 0 { 1 } else { w.div_ceil(width).max(1) }
        })
        .sum();
    count.max(1)
}

/// Compute the visual (wrapped) (row, col) for a character index in a
/// multi-line string rendered at the given `width`.
fn visual_cursor_pos(input: &str, cursor_char_idx: usize, width: u16) -> (u16, u16) {
    if width == 0 {
        return (0, 0);
    }
    let before: String = input.chars().take(cursor_char_idx).collect();
    let lines: Vec<&str> = before.split('\n').collect();
    let Some((cursor_line, complete)) = lines.split_last() else {
        return (0, 0);
    };

    // Complete logical lines before the cursor's line.
    let mut visual_row: u16 = complete
        .iter()
        .map(|line| {
            let w: u16 = line.chars().map(|c| c.width().unwrap_or(0) as u16).sum();
            if w == 0 { 1 } else { w.div_ceil(width).max(1) }
        })
        .sum();

    let cursor_line_width: u16 = cursor_line
        .chars()
        .map(|c| c.width().unwrap_or(0) as u16)
        .sum();
    if cursor_line_width == 0 {
        return (visual_row, 0);
    }
    visual_row += cursor_line_width / width;
    (visual_row, cursor_line_width % width)
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
    use super::*;

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
    fn test_visual_cursor_pos_wraps_and_counts_lines() {
        assert_eq!(visual_cursor_pos("abcdef", 6, 4), (1, 2));
        assert_eq!(visual_cursor_pos("ab\ncd", 4, 10), (1, 1));
        assert_eq!(visual_cursor_pos("anything", 3, 0), (0, 0));
    }
}
