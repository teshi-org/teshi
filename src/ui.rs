use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs};
use tui_tree_widget::Tree;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::agent::pipeline::GenerationStage;
use crate::app::{
    App, CaseDetail, ChangeKind, ClickableRegion, ColumnFocus, MainTab, MindMapFocus, RunStatus,
};
use crate::bdd_nav::nav_body_char_range_in_buffer;
use crate::gherkin_lang::{GherkinLanguage, StepKeywordType};
use crate::highlight::{StepHighlightState, highlight_line_with_state};
use crate::markdown::render_markdown;

/// Stage-2 preview: one solid style for the tree-selected line (avoids span-patch gaps that read as bright blocks).
const PREVIEW_CURSOR_BG: Color = Color::DarkGray;
const PREVIEW_CURSOR_FG: Color = Color::White;
const STATUS_PENDING: Color = Color::DarkGray;
const STATUS_RUNNING: Color = Color::Yellow;
const STATUS_PASSED: Color = Color::Green;
const STATUS_FAILED: Color = Color::Red;
const STATUS_SKIPPED: Color = Color::Gray;
const KEYWORD_GIVEN: Color = Color::Blue;
const KEYWORD_WHEN: Color = Color::Yellow;
const KEYWORD_THEN: Color = Color::Green;
const KEYWORD_AND: Color = Color::Gray;
const KEYWORD_BUT: Color = Color::Gray;
const EXPLORE_SELECTED_FOCUSED_BG: Color = Color::Rgb(16, 64, 168);
const EXPLORE_SELECTED_UNFOCUSED_BG: Color = Color::Rgb(125, 170, 242);
const STEP_KEYWORD_COL_WIDTH: usize = 6;
const SELECTION_BG: Color = Color::Rgb(64, 96, 160);
const SELECTION_FG: Color = Color::White;
const HIGHLIGHT_SELECTED_FG: Color = Color::Yellow;
const HIGHLIGHT_UNFOCUSED_FG: Color = Color::Cyan;

/// Braille spinner frames for the thinking indicator.
fn spinner_frame() -> &'static str {
    const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as usize;
    SPINNER[elapsed / 80 % SPINNER.len()]
}

/// Format token counts with thousands separators for display.
fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if (s.len() - i).is_multiple_of(3) && i > 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

/// Returns a short token-usage label for the input area, or empty if no tokens used yet.
fn token_usage_label(in_tokens: u64, out_tokens: u64) -> String {
    if in_tokens == 0 && out_tokens == 0 {
        return String::new();
    }
    format!(
        "in: {} · out: {}",
        fmt_count(in_tokens),
        fmt_count(out_tokens)
    )
}

fn selected_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .fg(HIGHLIGHT_SELECTED_FG)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(HIGHLIGHT_UNFOCUSED_FG)
    }
}

fn popup_highlight_block(title: &'static str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(Style::default().fg(EXPLORE_SELECTED_FOCUSED_BG))
        .border_style(Style::default().fg(EXPLORE_SELECTED_UNFOCUSED_BG))
}

/// Applies `patch` on UTF-8 character indices `[range.start, range.end)` within each span.
fn apply_patch_to_char_range(
    line: Line<'static>,
    range: std::ops::Range<usize>,
    patch: Style,
) -> Line<'static> {
    if range.start >= range.end {
        return line;
    }
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut idx = 0usize;
    let spans = line.spans;
    for span in spans {
        let text = span.content.to_string();
        let n = text.chars().count();
        let start_i = idx;
        let end_i = idx + n;
        let lo = range.start.max(start_i);
        let hi = range.end.min(end_i);
        if hi <= lo {
            out.push(span);
        } else {
            let lo_rel = lo - start_i;
            let hi_rel = hi - start_i;
            let chars: Vec<char> = text.chars().collect();
            if lo_rel > 0 {
                let before: String = chars[..lo_rel].iter().collect();
                out.push(Span::styled(before, span.style));
            }
            let mid: String = chars[lo_rel..hi_rel].iter().collect();
            let mid_style = span.style.patch(patch);
            out.push(Span::styled(mid, mid_style));
            if hi_rel < chars.len() {
                let after: String = chars[hi_rel..].iter().collect();
                out.push(Span::styled(after, span.style));
            }
        }
        idx = end_i;
    }
    Line::from(out)
}

/// Truncates styled spans so total display width does not exceed `max_cols` (Unicode columns).
///
/// When content is wider than the editor inner width, `pad_line_to_width` intentionally does not
/// pad; `Buffer::set_line` then clips visually while the `Line` still reports a larger width. That
/// mismatch breaks ratatui's terminal diff for trailing cells (observed as garbling on Windows).
fn truncate_line_to_cols(line: Line<'static>, max_cols: u16) -> Line<'static> {
    let max = max_cols as usize;
    if line.width() <= max {
        return line;
    }
    let line_style = line.style;
    let alignment = line.alignment;
    let mut budget = max;
    let mut out_spans: Vec<Span<'static>> = Vec::new();
    for span in line.spans {
        if budget == 0 {
            break;
        }
        let s = span.content.to_string();
        let mut acc = String::new();
        for ch in s.chars() {
            let w = ch.width().unwrap_or(0);
            if w == 0 {
                acc.push(ch);
                continue;
            }
            if w > budget {
                break;
            }
            acc.push(ch);
            budget -= w;
        }
        if !acc.is_empty() {
            out_spans.push(Span::styled(acc, span.style));
        }
        if budget == 0 {
            break;
        }
    }
    let mut out = Line::from(out_spans);
    out.style = line_style;
    out.alignment = alignment;
    out
}

/// Pads a line to `target_cols` display width using a trailing span (Unicode column widths).
fn pad_line_to_width(mut line: Line<'static>, target_cols: u16, trail: Style) -> Line<'static> {
    let t = target_cols as usize;
    let w = line.width();
    if w >= t {
        return line;
    }
    line.push_span(Span::styled(" ".repeat(t - w), trail));
    line
}

fn step_line_display(
    line: &str,
    in_doc_string: bool,
    lang: &GherkinLanguage,
) -> (String, usize, usize) {
    if in_doc_string {
        return (line.to_string(), 0, 0);
    }
    let trimmed = line.trim_start();
    let leading = line.len().saturating_sub(trimmed.len());
    let Some((keyword, _ty)) = lang.match_step_prefix(trimmed) else {
        return (line.to_string(), 0, 0);
    };
    let kw_len = keyword.chars().count();
    if kw_len >= STEP_KEYWORD_COL_WIDTH {
        return (line.to_string(), 0, 0);
    }
    let pad = STEP_KEYWORD_COL_WIDTH - kw_len;
    let mut out = String::new();
    let lead: String = line.chars().take(leading).collect();
    out.push_str(&lead);
    out.push_str(&" ".repeat(pad));
    out.push_str(trimmed);
    (out, pad, leading)
}

fn status_color(status: RunStatus) -> Color {
    match status {
        RunStatus::Idle => STATUS_PENDING,
        RunStatus::Running => STATUS_RUNNING,
        RunStatus::Passed => STATUS_PASSED,
        RunStatus::Failed => STATUS_FAILED,
        RunStatus::Skipped => STATUS_SKIPPED,
    }
}

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    app.clickable_regions.clear();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let top_tabs = Tabs::new(vec![
        Line::from(" Explore [1] "),
        Line::from(" MindMap [2] "),
        Line::from(" AI [3] "),
    ])
    .select(match app.active_tab {
        MainTab::Explore => 0,
        MainTab::MindMap => 1,
        MainTab::Ai => 2,
    })
    .style(Style::default().fg(Color::DarkGray))
    .highlight_style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )
    .divider(" ");
    frame.render_widget(top_tabs, chunks[0]);

    // Push tab regions for click/hover tracking
    app.clickable_regions
        .push(ClickableRegion::Tab(MainTab::Explore));
    app.clickable_regions
        .push(ClickableRegion::Tab(MainTab::MindMap));
    app.clickable_regions
        .push(ClickableRegion::Tab(MainTab::Ai));

    let divider_w = chunks[1].width as usize;
    let divider_line = "─".repeat(divider_w.max(1));
    frame.render_widget(
        Paragraph::new(divider_line).style(Style::default().fg(Color::DarkGray)),
        chunks[1],
    );

    render_main_panel(frame, app, chunks[2]);

    // Agent change approval status is shown only in the AI chat panel's status
    // bar (above the input area) — never in the bottom command line.
    if let Some(ref msg) = app.status_message {
        let status_line = Line::from(vec![Span::styled(
            msg.as_str(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]);
        frame.render_widget(Paragraph::new(status_line), chunks[3]);
    } else if app.active_tab == MainTab::Explore && !app.is_editor_active() {
        render_explore_footer(frame, app, chunks[3]);
    } else if app.active_tab == MainTab::Ai {
        render_ai_footer(frame, app, chunks[3]);
    } else {
        let key_hints = footer_hints(app);
        frame.render_widget(Paragraph::new(key_hints), chunks[3]);
    }

    render_external_change_prompt(frame, app, chunks[2]);

    // Change Summary overlay (MindMap tab)
    render_change_summary_panel(frame, app, chunks[2]);

    if app.auth_panel_active {
        render_auth_panel(frame, app, frame.area());
    }

    if app.model_panel_active {
        render_model_panel(frame, app, frame.area());
    }

    if app.session_panel_active {
        render_session_panel(frame, app, frame.area());
    }

    if app.quit_pending_confirm {
        render_quit_panel(frame, app, frame.area());
    }

    if app.approval_panel_active {
        render_approval_panel(frame, app, frame.area());
    }

    if app.agent_profile_panel_active {
        render_agent_profile_panel(frame, app, frame.area());
    }
}

fn render_main_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    match app.active_tab {
        MainTab::MindMap => render_mindmap_panel(frame, app, area),
        MainTab::Explore => render_explore_panel(frame, app, area),
        MainTab::Ai => render_ai_panel(frame, app, area),
    }
}

/// Renders the AI chat panel with a sidebar for agent management.
fn render_ai_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width < 25 || area.height < 3 {
        return;
    }

    // Layout: sidebar (left) + main panel (right)
    let [sidebar_area, main_area] =
        Layout::horizontal([Constraint::Length(28), Constraint::Min(10)]).areas(area);

    render_agent_sidebar(frame, app, sidebar_area);
    render_agent_chat(frame, app, main_area);
}

/// Renders the agent sidebar — list of agents with status indicators.
fn render_agent_sidebar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    use crate::app::AiStatus;
    if area.width < 5 || area.height < 3 {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Agents ")
        .style(Style::default());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, agent) in app.agents.iter().enumerate() {
        let is_selected = i == app.selected_agent;
        let prefix = if is_selected { "▸" } else { " " };
        let (status_char, status_color) = match agent.status {
            AiStatus::Waiting => ("●", Color::Yellow),
            AiStatus::AwaitingApproval => ("◆", Color::Cyan),
            AiStatus::Error => ("●", Color::Red),
            AiStatus::Idle => ("○", Color::Green),
        };
        let title = if agent.title.len() > 12 {
            format!("{}..", &agent.title[..12])
        } else {
            agent.title.clone()
        };
        let text = format!("{prefix} {status_char} {title}");
        let style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(status_color)
        };
        lines.push(Line::styled(text, style));
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled(
        " a new  x close ",
        Style::default().fg(Color::DarkGray),
    ));
    lines.push(Line::styled(
        " j/k switch",
        Style::default().fg(Color::DarkGray),
    ));

    let visible: Vec<Line<'static>> = lines.into_iter().take(inner.height as usize).collect();
    frame.render_widget(Paragraph::new(Text::from(visible)), inner);
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
    if lines.is_empty() {
        return (0, 0);
    }

    let mut visual_row: u16 = 0;

    // Complete logical lines before the cursor's line
    if lines.len() > 1 {
        for line in &lines[..lines.len() - 1] {
            let w: u16 = line.chars().map(|c| c.width().unwrap_or(0) as u16).sum();
            visual_row += if w == 0 { 1 } else { w.div_ceil(width).max(1) };
        }
    }

    // The partial line containing the cursor
    let cursor_line = lines[lines.len() - 1];
    let cursor_line_width: u16 = cursor_line
        .chars()
        .map(|c| c.width().unwrap_or(0) as u16)
        .sum();
    if cursor_line_width == 0 {
        return (visual_row, 0);
    }
    let col = cursor_line_width % width;
    visual_row += cursor_line_width / width;
    (visual_row, col)
}

/// Renders the active agent's chat panel.
fn render_agent_chat(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    use crate::app::{AiRole, AiStatus};

    if area.width < 10 || area.height < 3 {
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" AI Chat — {} ", app.active_agent_profile().name));
    let inner = block.inner(area);
    frame.render_widget(&block, area);

    // Render approval mode badge in the top-right corner
    let badge_label = format!("[{}]", app.approval_mode.display_name());
    let badge_width = badge_label.chars().count() as u16 + 2;
    let badge_x = area.right().saturating_sub(badge_width + 1);
    if badge_x > area.x {
        // Register clickable region
        app.clickable_regions.push(ClickableRegion::ApprovalBadge {
            row_y: area.y,
            col_x: badge_x,
            col_right: area.right().saturating_sub(1),
        });
        // Badge styling based on mode
        let mut badge_style = if app.approval_mode.requires_manual_approval() {
            Style::default().fg(Color::Yellow)
        } else if app.approval_mode.auto_accepts() {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        };
        // Hover effect: reverse video to indicate clickability
        if app.mouse_position.is_some_and(|(mx, my)| {
            my == area.y && mx >= badge_x && mx < area.right().saturating_sub(1)
        }) {
            badge_style = badge_style.add_modifier(Modifier::REVERSED);
        }
        let badge = Paragraph::new(Line::styled(badge_label, badge_style));
        let badge_area = Rect::new(badge_x, area.y, badge_width, 1);
        frame.render_widget(badge, badge_area);
    }

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Layout: chat history (top) + status bar + [suggestions] + input bar (bottom)
    let status_height: u16 = 1;

    // Dynamic input height: based on visual wrapped line count, grows with content
    let input_text_width = inner.width.saturating_sub(2); // Borders::ALL on input block
    let input_text_rows = visual_line_count_for_width(&app.agent().input, input_text_width)
        .min((inner.height / 3).max(3));
    let input_height: u16 = input_text_rows + 2; // +2 for top and bottom borders

    // Compute filtered suggestion count so height adapts to what's shown
    let filter = if app.slash_suggestion_active {
        app.agent()
            .input
            .strip_prefix('/')
            .unwrap_or("")
            .to_lowercase()
    } else {
        String::new()
    };
    let sugg_count = if app.slash_suggestion_active {
        crate::app::SLASH_COMMANDS
            .iter()
            .filter(|(name, _)| filter.is_empty() || name.starts_with(&filter))
            .count() as u16
    } else {
        0
    };

    let sugg_height: u16 = if app.slash_suggestion_active && sugg_count > 0 {
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
    let chat_area = Rect::new(inner.x, inner.y, inner.width, chat_height);
    let [chat_body, scrollbar_v] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).areas(chat_area);

    let status_area = Rect::new(inner.x, inner.y + chat_height, inner.width, status_height);
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

    // ── Chat history ────────────────────────────────────────────────
    let mut chat_lines: Vec<Line<'static>> = Vec::new();

    // Add a greeting if no messages
    if app.agent().messages.is_empty() {
        let greeting = Line::raw("Welcome to AI Chat! Type a message below and press Enter.");
        chat_lines.push(greeting);
        chat_lines.push(Line::raw(""));
        if !crate::llm::LlmConfig::is_configured() {
            chat_lines.push(
                Line::raw("Note: Set TESHI_LLM_API_KEY to enable AI responses.")
                    .style(Style::default().fg(Color::Yellow)),
            );
        }
    }

    for msg in &app.agent().messages {
        // Render tool result messages with a distinct style
        if matches!(msg.role, AiRole::Tool) {
            let is_error = msg.content.starts_with("Error:");
            let tool_id = msg.tool_call_id.as_deref().unwrap_or("?");
            let status_icon = if is_error { "✗" } else { "✓" };
            let status_color = if is_error {
                Color::Red
            } else {
                Color::DarkGray
            };
            chat_lines.push(
                Line::raw(format!("  {status_icon} Tool ({tool_id})")).style(
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
            );
            let md_lines = render_markdown(&msg.content);
            for md_line in md_lines {
                let mut spans = vec![Span::raw("    ")];
                spans.extend(
                    md_line
                        .spans
                        .into_iter()
                        .map(|s| Span::styled(s.content.to_string(), s.style)),
                );
                let mut line = Line::from(spans);
                line.style = md_line.style;
                chat_lines.push(line);
            }
            chat_lines.push(Line::raw(""));
            continue;
        }
        let prefix = match msg.role {
            AiRole::User => "▶ You",
            AiRole::Assistant => "▷ 🥰",
            _ => unreachable!(),
        };
        let role_color = match msg.role {
            AiRole::User => Color::Cyan,
            AiRole::Assistant => Color::Green,
            _ => unreachable!(),
        };
        // Show source tag for MindMap-initiated messages
        let source_tag = msg
            .source
            .as_ref()
            .map(|s| format!("[{s}] "))
            .unwrap_or_default();
        let prefix_text = format!("{source_tag}{prefix}:");
        chat_lines.push(
            Line::raw(prefix_text)
                .style(Style::default().fg(role_color).add_modifier(Modifier::BOLD)),
        );
        // Render message content as Markdown with a 2-space indent.
        if !msg.content.is_empty() {
            let md_lines = render_markdown(&msg.content);
            for md_line in md_lines {
                let mut spans = vec![Span::raw("  ")];
                spans.extend(
                    md_line
                        .spans
                        .into_iter()
                        .map(|s| Span::styled(s.content.to_string(), s.style)),
                );
                let mut line = Line::from(spans);
                line.style = md_line.style;
                chat_lines.push(line);
            }
        }
        // Render tool call blocks with status indicators
        if let Some(tcs) = &msg.tool_calls {
            for tc in tcs {
                let is_pending = app.agent().status == AiStatus::AwaitingApproval
                    && app
                        .pending_agent_changes
                        .iter()
                        .any(|c| c.tool_call_id == tc.id);
                let has_tool_result = app.agent().messages.iter().any(|m| {
                    matches!(m.role, AiRole::Tool) && m.tool_call_id.as_deref() == Some(&tc.id)
                });
                let is_error = has_tool_result
                    && app.agent().messages.iter().any(|m| {
                        matches!(m.role, AiRole::Tool)
                            && m.tool_call_id.as_deref() == Some(&tc.id)
                            && m.content.starts_with("Error:")
                    });
                let (icon, status_color) = if is_error {
                    ("✗", Color::Red)
                } else if has_tool_result {
                    ("✓", Color::Green)
                } else if is_pending {
                    ("◆", Color::Yellow)
                } else {
                    ("⏳", Color::Yellow)
                };
                let duration_str = tc
                    .execution_duration_ms
                    .map(|ms| {
                        if ms >= 1000 {
                            format!("{:.1}s", ms as f64 / 1000.0)
                        } else {
                            format!("{ms}ms")
                        }
                    })
                    .unwrap_or_default();
                let tool_line = if duration_str.is_empty() {
                    format!("  🔧 {}  {icon}", tc.name)
                } else {
                    format!("  🔧 {}  {icon} {duration_str}", tc.name)
                };
                chat_lines.push(
                    Line::raw(tool_line).style(
                        Style::default()
                            .fg(status_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                );
            }
        }
        chat_lines.push(Line::raw(""));
    }

    // ── Inline diff for pending changes ────────────────────────
    if app.agent().status == AiStatus::AwaitingApproval && !app.pending_change_diffs.is_empty() {
        let total_diffs = app.pending_change_diffs.len();
        let title = if total_diffs > 1 {
            format!(" Pending Change (1 of {total_diffs}) ")
        } else {
            " Pending Change ".to_string()
        };
        chat_lines.push(Line::raw(""));
        chat_lines.push(
            Line::raw(format!("  🔧{title}")).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        );
        if let Some(diff) = app.pending_change_diffs.first() {
            let max_lines = chat_body
                .height
                .saturating_sub(chat_lines.len() as u16)
                .clamp(3, 20) as usize;
            for dl in diff.iter().take(max_lines) {
                let prefix = match dl.kind {
                    ChangeKind::Added => "+",
                    ChangeKind::Modified => "~",
                    ChangeKind::Deleted => "-",
                    ChangeKind::Unchanged => " ",
                };
                let color = match dl.kind {
                    ChangeKind::Added => Color::Green,
                    ChangeKind::Modified => Color::Yellow,
                    ChangeKind::Deleted => Color::Red,
                    ChangeKind::Unchanged => Color::DarkGray,
                };
                chat_lines.push(Line::styled(
                    format!("    {} {}", prefix, dl.text),
                    Style::default().fg(color),
                ));
            }
        }
        chat_lines.push(Line::raw(""));
    }

    // Show streaming partial response as a live assistant message
    if !app.agent().partial_response.is_empty() {
        chat_lines.push(
            Line::raw("▷ 🥰:").style(
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        );
        let md_lines = render_markdown(&app.agent().partial_response);
        for md_line in md_lines {
            let mut spans = vec![Span::raw("  ")];
            spans.extend(
                md_line
                    .spans
                    .into_iter()
                    .map(|s| Span::styled(s.content.to_string(), s.style)),
            );
            let mut line = Line::from(spans);
            line.style = md_line.style;
            chat_lines.push(line);
        }
        // Append a blinking cursor
        let last_line = chat_lines.pop().unwrap_or(Line::raw(""));
        let mut spans: Vec<Span<'_>> = last_line.spans.into_iter().collect();
        spans.push(Span::styled(
            "▌",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::SLOW_BLINK),
        ));
        chat_lines.push(Line::from(spans));
        chat_lines.push(Line::raw(""));
    }

    // Slice chat history based on scroll offset (0 = show bottom)
    let total_lines = chat_lines.len();
    let max_start = total_lines.saturating_sub(chat_body.height as usize);
    let start = max_start.saturating_sub(app.agent().scroll_offset.min(max_start));
    let end = (start + chat_body.height as usize).min(total_lines);
    let visible_lines: Vec<Line<'static>> = chat_lines[start..end].to_vec();

    frame.render_widget(
        Paragraph::new(Text::from(visible_lines))
            .wrap(ratatui::widgets::Wrap { trim: false })
            .style(Style::default()),
        chat_body,
    );

    // ── Scrollbar ────────────────────────────────────────────────────
    if total_lines > chat_body.height as usize && scrollbar_v.width > 0 {
        let thumb_len = {
            let raw = chat_body.height as f64 * chat_body.height as f64 / total_lines as f64;
            (raw.ceil().max(1.0) as u16).min(chat_body.height)
        };
        let thumb_pos = if total_lines <= chat_body.height as usize {
            0
        } else {
            let scrollable = total_lines.saturating_sub(chat_body.height as usize);
            let track = chat_body.height.saturating_sub(thumb_len);
            (start * track as usize / scrollable.max(1)) as u16
        };

        let scrollbar_lines: Vec<Line<'static>> = (0..chat_body.height)
            .map(|i| {
                if i >= thumb_pos && i < thumb_pos + thumb_len {
                    Line::styled("█", Style::default().fg(Color::White).bg(Color::DarkGray))
                } else {
                    Line::styled("░", Style::default().fg(Color::DarkGray))
                }
            })
            .collect();

        frame.render_widget(Paragraph::new(Text::from(scrollbar_lines)), scrollbar_v);
    }

    // ── Status bar (thinking / model name) ─────────────────────────
    // Handle the new AwaitingApproval status
    let status_text: String = match app.agent().status {
        AiStatus::Waiting
            if app.agent().partial_response.is_empty() && app.agent().tool_status.is_some() =>
        {
            let spinner = spinner_frame();
            app.agent()
                .tool_status
                .clone()
                .unwrap_or_else(|| format!("{spinner} Teshi is thinking..."))
        }
        AiStatus::Waiting if app.agent().partial_response.is_empty() => {
            format!("{} Teshi is thinking...", spinner_frame())
        }
        AiStatus::AwaitingApproval => {
            if matches!(app.generation_stage, GenerationStage::Confirming) {
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
    // Pipeline stage indicator (append to status_text when active)
    // Skip when already showing in AwaitingApproval status
    let status_text = if !matches!(
        app.generation_stage,
        GenerationStage::Idle | GenerationStage::Complete
    ) && !matches!(app.agent().status, AiStatus::AwaitingApproval)
    {
        let stage_label = app.generation_stage.label();
        if status_text.is_empty() {
            stage_label.to_string()
        } else {
            format!("{status_text} · {stage_label}")
        }
    } else {
        status_text
    };
    let model_label = app.active_model_label.as_deref().unwrap_or("");

    if status_area.width > 0 {
        let model_w = model_label.len() as u16;
        let [status_left, status_right] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(model_w)])
                .areas(status_area);

        if !status_text.is_empty() {
            match app.agent().status {
                AiStatus::Error => {
                    let error_lines = vec![
                        Line::raw(status_text.clone()).style(Style::default().fg(Color::Red)),
                        Line::raw("Press Enter to retry or Esc to clear")
                            .style(Style::default().fg(Color::DarkGray)),
                    ];
                    frame.render_widget(Paragraph::new(Text::from(error_lines)), status_left);
                }
                _ => {
                    let st_style = match app.agent().status {
                        AiStatus::Waiting => Style::default().fg(Color::Yellow),
                        AiStatus::AwaitingApproval => Style::default().fg(Color::Cyan),
                        _ => Style::default().fg(Color::DarkGray),
                    };
                    frame.render_widget(
                        Paragraph::new(Text::from(Line::raw(status_text).style(st_style))),
                        status_left,
                    );
                }
            }
        }
        if !model_label.is_empty() {
            frame.render_widget(
                Paragraph::new(Text::from(
                    Line::raw(model_label).style(Style::default().fg(Color::DarkGray)),
                )),
                status_right,
            );
        }
    }

    // ── Slash command suggestions ────────────────────
    if app.slash_suggestion_active && sugg_height > 0 {
        let mut sugg_lines: Vec<Line<'static>> = Vec::new();

        // Filter commands based on what the user typed after "/"
        let filtered: Vec<&(&str, &str)> = crate::app::SLASH_COMMANDS
            .iter()
            .filter(|(name, _)| filter.is_empty() || name.starts_with(&filter))
            .collect();

        // Clamp selection to filtered list
        let selection = app
            .slash_suggestion_selection
            .min(filtered.len().saturating_sub(1));

        for (i, (name, desc)) in filtered.iter().enumerate() {
            let is_selected = i == selection;
            let prefix = if is_selected { " ▸ " } else { "   " };
            let text = format!("{prefix}/{name}  {desc}");
            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            sugg_lines.push(Line::styled(text, style));
        }

        // Hint line
        sugg_lines.push(Line::styled(
            " ↑↓ navigate · Enter/Tab select · Esc close ",
            Style::default().fg(Color::DarkGray),
        ));

        let visible: Vec<Line<'static>> =
            sugg_lines.into_iter().take(sugg_height as usize).collect();
        frame.render_widget(
            Paragraph::new(Text::from(visible)).style(Style::default()),
            sugg_area,
        );
    }

    // ── Input bar ───────────────────────────────────────────────────
    let input_border_style = if app.ai_input_focused {
        Style::default()
            .fg(HIGHLIGHT_SELECTED_FG)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let label = token_usage_label(
        app.agent().total_input_tokens,
        app.agent().total_output_tokens,
    );
    let input_block = if label.is_empty() {
        Block::default()
            .borders(Borders::ALL)
            .border_style(input_border_style)
    } else {
        Block::default()
            .borders(Borders::ALL)
            .border_style(input_border_style)
            .title_top(Line::raw(label).alignment(Alignment::Right))
    };
    let input_inner = input_block.inner(input_area);
    frame.render_widget(input_block, input_area);

    let input_display: Text<'static> = if app.agent().input.is_empty() {
        if app.ai_input_focused {
            Text::raw("Type your message...")
        } else {
            Text::raw("")
        }
    } else {
        // Build lines explicitly so Paragraph renders multi-line correctly
        let raw = app.agent().input.as_str();
        let lines: Vec<Line<'static>> = raw
            .lines()
            .map(|l| Line::from(Span::raw(l.to_string())))
            .collect();
        Text::from(lines)
    };
    frame.render_widget(
        Paragraph::new(input_display)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .style(match app.agent().status {
                AiStatus::Waiting => Style::default().fg(Color::DarkGray),
                _ => Style::default(),
            }),
        input_inner,
    );

    // Show a visible cursor at the insertion point, accounting for wrapping.
    if app.ai_input_focused && app.agent().status != AiStatus::Waiting {
        let (vis_row, vis_col) = visual_cursor_pos(
            &app.agent().input,
            app.agent().input_cursor,
            input_inner.width,
        );
        frame.set_cursor_position((input_inner.x + vis_col, input_inner.y + vis_row));
    }
}

/// Renders the footer bar for the AI tab.
fn render_ai_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let send_label = if app.ai_input_focused {
        " Send [Enter] "
    } else {
        " Focus [Enter] "
    };
    let mut hint_spans = vec![
        footer_pill(send_label),
        Span::raw(" "),
        footer_pill(" Clear [Esc] "),
        Span::raw(" "),
        footer_pill(" Model [m] "),
    ];
    if !app.ai_input_focused {
        hint_spans.push(Span::raw(" "));
        hint_spans.push(footer_pill(" Quit [q] "));
    }
    // Show tool execution status in the footer when the agent is acting
    if let Some(ref tool_status) = app.agent().tool_status {
        hint_spans.push(Span::raw("  "));
        hint_spans.push(Span::styled(
            tool_status.clone(),
            Style::default().fg(Color::Yellow),
        ));
    }
    let hints = Line::from(hint_spans);
    frame.render_widget(Paragraph::new(hints), area);
}

/// Renders the MindMap layout: tree (55%) + right panel (45%) split vertically into
/// scenario preview (top) and AI chat (bottom).
fn render_mindmap_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if app.mindmap_ai_panel_visible && area.width >= 30 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);
        let tree_focused = app.mindmap_focus == MindMapFocus::Main;
        render_tree_panel(frame, app, cols[0], tree_focused);

        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(cols[1]);

        let preview_focused = app.mindmap_focus == MindMapFocus::AiPanel;
        render_mindmap_scenario_preview(frame, app, right_chunks[0], preview_focused);
        render_mindmap_agent_chat(frame, app, right_chunks[1]);
    } else {
        render_tree_panel(frame, app, area, true);
    }
}

/// Renders the Explore tab: three-column feature/scenario/step browser.
fn render_explore_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if app.explore_edit_mode {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(area);
        render_editor_panel(frame, app, cols[0], false);
        render_reserved_panel(frame, app, cols[1]);
        return;
    }
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(30),
            Constraint::Percentage(50),
        ])
        .split(area);

    render_explore_features(frame, app, cols[0]);
    render_explore_scenarios(frame, app, cols[1]);
    render_explore_steps(frame, app, cols[2]);
    render_failure_detail(frame, app, area);
}

fn explore_select_style(focused: bool) -> Style {
    selected_style(focused)
}

fn explore_block(title: &str, focused: bool) -> Block<'_> {
    let title_style = if focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(title_style)
}

fn feature_display_name(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "feature".to_string())
}

fn truncate_string_to_cols(text: &str, max_cols: u16) -> String {
    let max = max_cols as usize;
    let mut budget = max;
    let mut out = String::new();
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if w == 0 {
            out.push(ch);
            continue;
        }
        if w > budget {
            break;
        }
        out.push(ch);
        budget -= w;
    }
    out
}

fn render_explore_features(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let focused = app.explore_focus == ColumnFocus::Feature;
    let block = explore_block("Features", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let highlight_style = explore_select_style(focused);
    let normal = Style::default();
    let mut lines: Vec<Line> = Vec::new();

    if app.project.features.is_empty() {
        lines.push(Line::styled(
            " (no features)",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        for (i, feature) in app.project.features.iter().enumerate() {
            let label = feature_display_name(&feature.file_path);
            let style = if i == app.explore_selected_feature {
                highlight_style
            } else {
                normal
            };
            let mut line = Line::from(Span::styled(format!(" {label}"), style));
            line = truncate_line_to_cols(line, inner.width);
            let trail = if i == app.explore_selected_feature {
                highlight_style
            } else {
                Style::default()
            };
            line = pad_line_to_width(line, inner.width, trail);
            lines.push(line);
        }
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);

    // Register clickable regions for each feature row
    for (i, _feature) in app.project.features.iter().enumerate() {
        app.clickable_regions.push(ClickableRegion::ExploreFeature {
            feature_idx: i,
            row_y: inner.y + i as u16,
            col_x: inner.x,
            col_right: inner.right(),
        });
    }
}

fn render_explore_scenarios(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let focused = app.explore_focus == ColumnFocus::Scenario;
    let scenarios_title = explore_scenarios_title(app);
    let block = explore_block(scenarios_title.as_str(), focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let normal = Style::default();
    let mut lines: Vec<Line> = Vec::new();

    let scenarios = app
        .project
        .features
        .get(app.explore_selected_feature)
        .map(|f| &f.scenarios);

    if scenarios.is_none_or(|s| s.is_empty()) {
        lines.push(Line::styled(
            " (no scenarios)",
            Style::default().fg(Color::DarkGray),
        ));
    } else if let Some(scenarios) = scenarios {
        for (i, scenario) in scenarios.iter().enumerate() {
            let status = app
                .explore_case_status
                .get(&(app.explore_selected_feature, i))
                .copied()
                .unwrap_or(RunStatus::Idle);
            let status_dot = Span::styled("●", Style::default().fg(status_color(status)));
            let name = Span::styled(format!(" {}", scenario.name), normal);
            let mut line = Line::from(vec![status_dot, name]);
            if i == app.explore_selected_scenario {
                line = apply_line_background(line, explore_select_style(focused));
            }
            line = truncate_line_to_cols(line, inner.width);
            let trail = if i == app.explore_selected_scenario {
                explore_select_style(focused)
            } else {
                Style::default()
            };
            line = pad_line_to_width(line, inner.width, trail);
            lines.push(line);
        }
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);

    // Register clickable regions for each scenario row
    if let Some(scenarios) = app
        .project
        .features
        .get(app.explore_selected_feature)
        .map(|f| &f.scenarios)
    {
        for (i, _scenario) in scenarios.iter().enumerate() {
            app.clickable_regions
                .push(ClickableRegion::ExploreScenario {
                    scenario_idx: i,
                    row_y: inner.y + i as u16,
                    col_x: inner.x,
                    col_right: inner.right(),
                });
        }
    }
}

fn explore_scenarios_title(app: &App) -> String {
    let count = app
        .project
        .features
        .get(app.explore_selected_feature)
        .map(|f| f.scenarios.len())
        .unwrap_or(0);
    format!("Scenarios ({count})")
}

const DIFF_ADDED_BG: Color = Color::Rgb(24, 72, 36);
const DIFF_MODIFIED_BG: Color = Color::Rgb(68, 56, 16);
const DIFF_DELETED_BG: Color = Color::Rgb(72, 24, 24);

fn render_explore_steps(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let focused = app.explore_focus == ColumnFocus::Step;
    if app.explore_edit_mode {
        render_editor_panel(frame, app, area, false);
        return;
    }

    // ── Diff mode (from pending agent change) ──────────────────────────
    if let Some(ref diff_lines) = app.explore_diff_lines {
        let block = explore_block("Steps (diff)", focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let feature = app.project.features.get(app.explore_selected_feature);
        let scenario = feature.and_then(|f| f.scenarios.get(app.explore_selected_scenario));
        let scenario_steps = scenario.map(|s| s.steps.as_slice()).unwrap_or(&[]);

        // Determine line-number range of the current scenario's steps
        let first_line = scenario_steps.first().map(|s| s.line_number).unwrap_or(0);
        let last_line = scenario_steps.last().map(|s| s.line_number).unwrap_or(0);

        let mut lines: Vec<Line> = Vec::new();

        // Diff-mode header
        lines.push(Line::styled(
            format!(" {} changed line(s) — [D] close", diff_lines.len()),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::raw(""));

        // Filter diff lines to those relevant to the current scenario
        let relevant: Vec<&crate::app::DiffLine> = diff_lines
            .iter()
            .filter(|dl| {
                dl.line_number_1based == 0
                    || (dl.line_number_1based >= first_line && dl.line_number_1based <= last_line)
            })
            .collect();

        if relevant.is_empty() {
            lines.push(Line::styled(
                " (no changes in this scenario)",
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            for dl in relevant {
                let (gutter_ch, gutter_color, bg_color) = match dl.kind {
                    crate::app::ChangeKind::Added => ("+", Color::Green, DIFF_ADDED_BG),
                    crate::app::ChangeKind::Modified => ("~", Color::Yellow, DIFF_MODIFIED_BG),
                    crate::app::ChangeKind::Deleted => ("-", Color::Red, DIFF_DELETED_BG),
                    crate::app::ChangeKind::Unchanged => (" ", Color::DarkGray, Color::Reset),
                };
                let line_style = Style::default().bg(bg_color);
                let gutter_style = Style::default()
                    .fg(gutter_color)
                    .add_modifier(Modifier::BOLD)
                    .bg(bg_color);
                let text_style = Style::default().bg(bg_color);
                let gutter = Span::styled(format!("{gutter_ch} "), gutter_style);
                let text = Span::styled(dl.text.clone(), text_style);
                let mut line = Line::from(vec![gutter, text]);
                line = truncate_line_to_cols(line, inner.width);
                line = pad_line_to_width(line, inner.width, line_style);
                lines.push(line);
            }
        }

        frame.render_widget(Paragraph::new(Text::from(lines)), inner);
        return;
    }

    // ── Normal (browse) mode ──────────────────────────────────────────
    let block = explore_block("Steps", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let highlight_style = explore_select_style(focused);
    let mut lines: Vec<Line> = Vec::new();
    let mut line_idx = inner.y;

    let feature = app.project.features.get(app.explore_selected_feature);
    let scenario = feature.and_then(|f| f.scenarios.get(app.explore_selected_scenario));
    let background_steps = feature
        .and_then(|f| f.background.as_ref())
        .map(|bg| bg.steps.as_slice())
        .unwrap_or(&[]);
    let scenario_steps = scenario.map(|s| s.steps.as_slice()).unwrap_or(&[]);

    if background_steps.is_empty() && scenario_steps.is_empty() {
        lines.push(Line::styled(
            " (no steps)",
            Style::default().fg(Color::DarkGray),
        ));
        line_idx += 1;
    } else {
        let mut last_major: Option<Color> = None;
        if !background_steps.is_empty() {
            lines.push(Line::styled(
                " Background:",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ));
            line_idx += 1;
            for step in background_steps {
                let kw = format!("{:>6}", step.keyword);
                let kw_color = match step.keyword_type {
                    StepKeywordType::Given => {
                        last_major = Some(KEYWORD_GIVEN);
                        KEYWORD_GIVEN
                    }
                    StepKeywordType::When => {
                        last_major = Some(KEYWORD_WHEN);
                        KEYWORD_WHEN
                    }
                    StepKeywordType::Then => {
                        last_major = Some(KEYWORD_THEN);
                        KEYWORD_THEN
                    }
                    StepKeywordType::And => last_major.unwrap_or(KEYWORD_AND),
                    StepKeywordType::But => last_major.unwrap_or(KEYWORD_BUT),
                };
                let kw_span = Span::styled(kw, Style::default().fg(kw_color));
                let body_span = Span::styled(
                    format!(" {}", step.text),
                    Style::default().fg(Color::DarkGray),
                );
                let mut line = Line::from(vec![kw_span, body_span]);
                line = truncate_line_to_cols(line, inner.width);
                line = pad_line_to_width(line, inner.width, Style::default());
                lines.push(line);
                line_idx += 1;
            }
            lines.push(Line::raw(""));
            line_idx += 1;
        }

        if let Some(scenario) = scenario
            && !scenario.tags.is_empty()
        {
            lines.push(Line::styled(
                format!(" {}", scenario.tags.join(" ")),
                Style::default().fg(Color::DarkGray),
            ));
            line_idx += 1;
        }

        for (i, step) in scenario_steps.iter().enumerate() {
            let kw = format!("{:>6}", step.keyword);
            let kw_color = match step.keyword_type {
                StepKeywordType::Given => {
                    last_major = Some(KEYWORD_GIVEN);
                    KEYWORD_GIVEN
                }
                StepKeywordType::When => {
                    last_major = Some(KEYWORD_WHEN);
                    KEYWORD_WHEN
                }
                StepKeywordType::Then => {
                    last_major = Some(KEYWORD_THEN);
                    KEYWORD_THEN
                }
                StepKeywordType::And => last_major.unwrap_or(KEYWORD_AND),
                StepKeywordType::But => last_major.unwrap_or(KEYWORD_BUT),
            };
            let kw_span = Span::styled(kw, Style::default().fg(kw_color));
            let body_span = if i == app.explore_selected_step {
                Span::styled(format!(" {}", step.text), highlight_style)
            } else {
                Span::raw(format!(" {}", step.text))
            };
            let mut line = Line::from(vec![kw_span, body_span]);
            line = truncate_line_to_cols(line, inner.width);
            let trail = if i == app.explore_selected_step {
                highlight_style
            } else {
                Style::default()
            };
            line = pad_line_to_width(line, inner.width, trail);
            lines.push(line);
            line_idx += 1;
            app.clickable_regions.push(ClickableRegion::ExploreStep {
                step_idx: i,
                row_y: line_idx - 1,
                col_x: inner.x,
                col_right: inner.right(),
            });
        }

        if let Some(scenario) = scenario
            && !scenario.examples.is_empty()
        {
            lines.push(Line::raw(""));
            line_idx += 1;
            for table in &scenario.examples {
                if !table.tags.is_empty() {
                    lines.push(Line::styled(
                        format!(" {}", table.tags.join(" ")),
                        Style::default().fg(Color::DarkGray),
                    ));
                    line_idx += 1;
                }
                lines.push(Line::styled(
                    " Examples:",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ));
                line_idx += 1;
                for row in render_examples_table_lines(&table.headers, &table.rows) {
                    lines.push(Line::raw(format!("   {row}")));
                    line_idx += 1;
                }
            }
        }
    }

    let _ = line_idx;
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn render_examples_table_lines(headers: &[String], rows: &[Vec<String>]) -> Vec<String> {
    if headers.is_empty() {
        return Vec::new();
    }
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|h| UnicodeWidthStr::width(h.as_str()))
        .collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(i) {
                *width = (*width).max(UnicodeWidthStr::width(cell.as_str()));
            }
        }
    }
    let format_row = |cells: &[String]| {
        let mut out = String::from("|");
        for (i, width) in widths.iter().enumerate() {
            let cell = cells.get(i).map_or("", String::as_str);
            let cell_w = UnicodeWidthStr::width(cell);
            let pad = width.saturating_sub(cell_w);
            out.push(' ');
            out.push_str(cell);
            out.push_str(&" ".repeat(pad));
            out.push(' ');
            out.push('|');
        }
        out
    };

    let mut out = Vec::with_capacity(rows.len() + 2);
    out.push(format_row(headers));
    for row in rows {
        out.push(format_row(row));
    }
    out
}

fn render_failure_detail(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if !app.explore_detail_open {
        return;
    }
    let Some((fi, si)) = app.explore_detail_case else {
        return;
    };
    let Some(detail) = app.explore_case_details.get(&(fi, si)) else {
        return;
    };
    if detail.status != RunStatus::Failed {
        return;
    }
    let Some(feature) = app.project.features.get(fi) else {
        return;
    };
    let Some(scenario) = feature.scenarios.get(si) else {
        return;
    };

    let popup_w = (area.width as f32 * 0.75) as u16;
    let popup_h = (area.height as f32 * 0.70) as u16;
    let popup_w = popup_w.max(20).min(area.width);
    let popup_h = popup_h.max(10).min(area.height);
    let popup_x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(popup_x, popup_y, popup_w, popup_h);

    frame.render_widget(Clear, popup);
    let block = popup_highlight_block("Failure Details");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let out_lines = truncate_lines(build_case_detail_lines(&scenario.name, detail), inner.width);
    frame.render_widget(Paragraph::new(Text::from(out_lines)), inner);
}

fn render_external_change_prompt(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if !app.has_external_change_prompt() {
        return;
    }
    if area.width < 10 || area.height < 3 {
        return;
    }
    let title = app
        .external_change_prompt_title()
        .unwrap_or("Feature changed on disk");
    let file_name = app
        .external_change_prompt_path()
        .unwrap_or_else(|| "feature".to_string());

    let popup_w = (area.width as f32 * 0.60) as u16;
    let popup_h = area.height.clamp(3, 7);
    let popup_w = popup_w.max(30).min(area.width);
    let popup_x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(popup_x, popup_y, popup_w, popup_h);

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(Style::default().fg(EXPLORE_SELECTED_FOCUSED_BG))
        .border_style(Style::default().fg(EXPLORE_SELECTED_UNFOCUSED_BG));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let lines = vec![
        Line::raw(format!("Detected external updates for {file_name}.")),
        Line::raw(""),
        Line::raw("Reload latest input: [Enter] / [r]"),
        Line::raw("Keep local buffer: [Esc] / [k]"),
    ];
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Renders the Change Summary overlay with pending added/modified/deleted nodes.
fn render_change_summary_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if !app.change_summary_visible || app.pending_change_summary.is_empty() {
        return;
    }
    if area.width < 20 || area.height < 5 {
        return;
    }

    let popup_w = (area.width as f32 * 0.65) as u16;
    let popup_h = (area.height as f32 * 0.55) as u16;
    let popup_w = popup_w.max(30).min(area.width);
    let popup_h = popup_h.max(10).min(area.height);
    let popup_x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(popup_x, popup_y, popup_w, popup_h);

    frame.render_widget(Clear, popup);
    let block = popup_highlight_block("Change Summary");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width < 10 || inner.height < 3 {
        return;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Header
    let header = format!(
        " {} change(s) pending — ↑↓ navigate, Enter jump, Esc close",
        app.pending_change_summary.len()
    );
    lines.push(Line::styled(header, Style::default().fg(Color::DarkGray)));
    lines.push(Line::raw(""));

    // List each change node
    for (i, node) in app.pending_change_summary.iter().enumerate() {
        let marker = if i == app.change_summary_selection {
            "▸ "
        } else {
            "  "
        };
        let (label, label_color) = match node.kind {
            crate::app::ChangeKind::Added => ("[+]", Color::Green),
            crate::app::ChangeKind::Modified => ("[~]", Color::Yellow),
            crate::app::ChangeKind::Deleted => ("[-]", Color::Red),
            crate::app::ChangeKind::Unchanged => continue,
        };
        let label_span = Span::styled(
            format!("{}{} ", marker, label),
            Style::default()
                .fg(label_color)
                .add_modifier(Modifier::BOLD),
        );
        let scenario_span = Span::styled(
            format!(" {}  ", node.scenario_name),
            Style::default().fg(Color::Cyan),
        );
        let step_span = Span::raw(node.step_text.clone());
        let mut line = Line::from(vec![label_span, scenario_span, step_span]);
        // Highlight selected row
        if i == app.change_summary_selection {
            line = apply_line_background(line, Style::default().bg(Color::Rgb(30, 30, 60)));
        }
        line = truncate_line_to_cols(line, inner.width);
        line = pad_line_to_width(line, inner.width, Style::default());
        lines.push(line);
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn apply_line_background(line: Line<'static>, bg: Style) -> Line<'static> {
    let mut out_spans: Vec<Span<'static>> = Vec::new();
    for span in line.spans {
        let text = span.content.to_string();
        let style = span.style.patch(bg);
        out_spans.push(Span::styled(text, style));
    }
    let mut out = Line::from(out_spans);
    out.style = line.style.patch(bg);
    out.alignment = line.alignment;
    out
}

/// Renders the collapsible tree using `tui-tree-widget`.
fn render_tree_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect, focused: bool) {
    let items = &app.mindmap_index.items;

    let highlight_style = selected_style(true);

    // Build title with indicators when highlights/filter are active
    let mut title_parts: Vec<&str> = vec!["MindMap"];
    if app.mindmap_index.has_active_filter() {
        title_parts.push("[filtered]");
    }
    if app.mindmap_index.has_active_highlights() {
        title_parts.push("[highlighted]");
    }
    let title = title_parts.join(" ");

    let block = if focused {
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
    } else {
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(Style::default().fg(Color::DarkGray))
    };

    let block_inner = block.inner(area);
    let tree = Tree::new(items)
        .expect("tree construction should succeed")
        .block(block)
        .highlight_style(highlight_style);

    app.tree_panel_rect = Some(block_inner);
    app.clickable_regions.push(ClickableRegion::Tree);

    frame.render_stateful_widget(tree, area, &mut app.tree_state);
}

/// Renders the scenario preview panel in the MindMap tab (top-right).
/// Shows the complete scenario/background text for the selected tree node.
fn render_mindmap_scenario_preview(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    focused: bool,
) {
    if area.width < 10 || area.height < 3 {
        return;
    }

    let buffer = app.preview_buffer.as_ref().unwrap_or(&app.buffer);

    let title = if app.preview_title.is_empty() {
        "Preview"
    } else {
        app.preview_title.as_str()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(if focused {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    frame.render_widget(Clear, inner);

    // Paint full-width spaces first to avoid stale pixels from previous frames
    let buf = frame.buffer_mut();
    for i in 0..inner.height {
        let y = inner.y.saturating_add(i);
        if y >= inner.bottom() {
            break;
        }
        buf.set_string(
            inner.x,
            y,
            " ".repeat(inner.width as usize),
            Style::default(),
        );
    }

    // ── Scenario location dropdown ───────────────────────────────────
    if app.scenario_dropdown_open
        && let Some(id) = crate::mindmap::selected_node_id(&app.tree_state)
        && let Some(locations) = app.mindmap_index.locations_for(id)
        && !locations.is_empty()
    {
        render_scenario_dropdown(frame, app, inner, locations);
        return; // dropdown replaces preview content
    }

    let cursor_row = app.preview_cursor_row;
    let visible_lines = inner.height as usize;

    // Compute scroll: center the cursor row
    let line_count = buffer.line_count();
    let max_scroll = line_count.saturating_sub(visible_lines);
    let actual_scroll = if cursor_row < visible_lines / 2 || line_count <= visible_lines {
        0
    } else {
        (cursor_row - visible_lines / 2).min(max_scroll)
    };
    app.preview_scroll_row = actual_scroll;

    // Render lines with Gherkin syntax highlighting
    let mut step_state = crate::highlight::StepHighlightState::default();
    let preview_style = Style::default().fg(Color::Cyan);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(visible_lines);

    for i in 0..visible_lines {
        let buf_row = actual_scroll + i;
        if buf_row >= line_count {
            lines.push(pad_line_to_width(
                Line::raw(String::new()),
                inner.width,
                Style::default(),
            ));
            continue;
        }
        let line = buffer.line(buf_row);
        let (display_line, _pad_offset, _pad_start) =
            step_line_display(&line, step_state.in_doc_string, buffer.language());
        let mut styled =
            highlight_line_with_state(&display_line, &mut step_state, buffer.language());

        if buf_row == cursor_row {
            styled = Line::from(Span::styled(display_line.to_string(), preview_style));
        }

        styled = truncate_line_to_cols(styled, inner.width);
        let pad_trail = if buf_row == cursor_row {
            preview_style
        } else {
            Style::default()
        };
        styled = pad_line_to_width(styled, inner.width, pad_trail);
        lines.push(styled);
    }

    for (i, line) in lines.iter().enumerate() {
        let y = inner.y.saturating_add(i as u16);
        if y >= inner.bottom() {
            break;
        }
        buf.set_line(inner.x, y, line, inner.width);
    }
}

/// Renders the scenario location dropdown inside the preview panel.
fn render_scenario_dropdown(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    locations: &[crate::mindmap::NodeLocation],
) {
    use crate::mindmap::LocationContext;

    let count = locations.len();
    let selection = app.scenario_dropdown_selection.min(count.saturating_sub(1));
    let max_items = (area.height as usize).saturating_sub(2).min(count);
    let list_height = (max_items + 2).min(area.height as usize) as u16; // +2 for borders

    let dropdown_area = Rect::new(area.x, area.y, area.width, list_height.min(area.height));
    if dropdown_area.height < 3 {
        return;
    }

    // Build item labels
    let mut items: Vec<String> = Vec::with_capacity(count);
    for loc in locations.iter().take(count) {
        let feature_name = app
            .project
            .features
            .get(loc.feature_idx)
            .and_then(|f| f.file_path.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("feature_{}", loc.feature_idx));

        let label = match loc.context {
            LocationContext::Scenario(sci) => {
                let scenario_name = app
                    .project
                    .features
                    .get(loc.feature_idx)
                    .and_then(|f| f.scenarios.get(sci))
                    .map(|s| s.name.as_str())
                    .unwrap_or("?");
                format!("{}: Scenario: {}", feature_name, scenario_name)
            }
            LocationContext::Background => {
                format!("{}: Background", feature_name)
            }
        };
        items.push(label);
    }

    // Render dropdown block
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Open Scenario")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(dropdown_area);
    frame.render_widget(Clear, dropdown_area);
    frame.render_widget(block, dropdown_area);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(max_items);
    let visible_items = inner.height as usize;
    let scroll_start = selection.saturating_sub(visible_items.saturating_sub(1) / 2);
    let scroll_end = (scroll_start + visible_items).min(items.len());

    for (i, item) in items[scroll_start..scroll_end].iter().enumerate() {
        let idx = scroll_start + i;
        let is_selected = idx == selection;
        let prefix = if is_selected { "▸ " } else { "  " };
        let style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let text = truncate_str(item, inner.width.saturating_sub(2) as usize);
        lines.push(Line::styled(format!("{}{}", prefix, text), style));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);

    // Hint line at the bottom of the dropdown area
    if dropdown_area.y + dropdown_area.height < area.y + area.height {
        let hint_y = dropdown_area.y + dropdown_area.height;
        let hint_line = Line::styled(
            " j/k navigate · Enter select · Esc close ",
            Style::default().fg(Color::DarkGray),
        );
        frame.render_widget(
            Paragraph::new(hint_line),
            Rect::new(area.x, hint_y, area.width, 1),
        );
    }
}

/// Truncate a string to fit within `max_width` columns (Unicode-aware).
fn truncate_str(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let width = s.width();
    if width <= max_width {
        return s.to_string();
    }
    let mut out = String::with_capacity(max_width);
    let mut w = 0;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if w + cw + 1 > max_width {
            out.push('…');
            break;
        }
        out.push(ch);
        w += cw;
    }
    out
}

/// Renders the AI chat panel in the MindMap tab (bottom-right).
fn render_mindmap_agent_chat(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width < 10 || area.height < 3 {
        return;
    }
    let focused = app.mindmap_focus == MindMapFocus::AiPanel;
    render_agent_chat_inner(frame, app, area, focused);
}

/// Core chat UI used by both the AI tab (full) and the MindMap tab (bottom panel).
/// `focused` controls border highlighting and input activation.
pub(crate) fn render_agent_chat_inner(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    focused: bool,
) {
    use crate::app::{AiRole, AiStatus};

    if area.width < 10 || area.height < 3 {
        return;
    }

    let border_style = if focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" AI Chat — {} ", app.active_agent_profile().name))
        .border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(&block, area);

    // Render approval badge in top-right corner
    let badge_label = format!("[{}]", app.approval_mode.display_name());
    let badge_width = badge_label.chars().count() as u16 + 2;
    let badge_x = area.right().saturating_sub(badge_width + 1);
    if badge_x > area.x {
        app.clickable_regions.push(ClickableRegion::ApprovalBadge {
            row_y: area.y,
            col_x: badge_x,
            col_right: area.right().saturating_sub(1),
        });
        let mut badge_style = if app.approval_mode.requires_manual_approval() {
            Style::default().fg(Color::Yellow)
        } else if app.approval_mode.auto_accepts() {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        };
        // Hover effect: reverse video to indicate clickability
        if app.mouse_position.is_some_and(|(mx, my)| {
            my == area.y && mx >= badge_x && mx < area.right().saturating_sub(1)
        }) {
            badge_style = badge_style.add_modifier(Modifier::REVERSED);
        }
        let badge = Paragraph::new(Line::styled(badge_label, badge_style));
        let badge_area = Rect::new(badge_x, area.y, badge_width, 1);
        frame.render_widget(badge, badge_area);
    }

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Layout: chat history (top) + status bar + input bar (bottom)
    let status_height: u16 = 1;

    let input_text_width = inner.width.saturating_sub(2); // Borders::ALL on input block
    let input_text_rows = visual_line_count_for_width(&app.agent().input, input_text_width)
        .min((inner.height / 3).max(3));
    let input_height: u16 = input_text_rows + 2;

    let chat_height = inner.height.saturating_sub(status_height + input_height);
    let chat_area = Rect::new(inner.x, inner.y, inner.width, chat_height);
    let status_area = Rect::new(inner.x, inner.y + chat_height, inner.width, status_height);
    let input_area = Rect::new(
        inner.x,
        inner.y + chat_height + status_height,
        inner.width,
        input_height,
    );

    // ── Chat history ────────────────────────────────────────────────
    let mut chat_lines: Vec<Line<'static>> = Vec::new();

    if app.agent().messages.is_empty() {
        chat_lines.push(Line::raw("Welcome to AI Chat!"));
        chat_lines.push(Line::raw(""));
        if !crate::llm::LlmConfig::is_configured() {
            chat_lines.push(
                Line::raw("Note: Set TESHI_LLM_API_KEY to enable AI responses.")
                    .style(Style::default().fg(Color::Yellow)),
            );
        }
    }

    for msg in &app.agent().messages {
        // Render tool result messages with a distinct style
        if matches!(msg.role, AiRole::Tool) {
            let is_error = msg.content.starts_with("Error:");
            let tool_id = msg.tool_call_id.as_deref().unwrap_or("?");
            let status_icon = if is_error { "✗" } else { "✓" };
            let status_color = if is_error {
                Color::Red
            } else {
                Color::DarkGray
            };
            chat_lines.push(
                Line::raw(format!("  {status_icon} Tool ({tool_id})")).style(
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
            );
            let md_lines = render_markdown(&msg.content);
            for md_line in md_lines {
                let mut spans = vec![Span::raw("    ")];
                spans.extend(
                    md_line
                        .spans
                        .into_iter()
                        .map(|s| Span::styled(s.content.to_string(), s.style)),
                );
                let mut line = Line::from(spans);
                line.style = md_line.style;
                chat_lines.push(line);
            }
            chat_lines.push(Line::raw(""));
            continue;
        }
        let prefix = match msg.role {
            AiRole::User => "▶ You",
            AiRole::Assistant => "▷ 🥰",
            _ => unreachable!(),
        };
        let role_color = match msg.role {
            AiRole::User => Color::Cyan,
            AiRole::Assistant => Color::Green,
            _ => unreachable!(),
        };
        let source_tag = msg
            .source
            .as_ref()
            .map(|s| format!("[{s}] "))
            .unwrap_or_default();
        let prefix_text = format!("{source_tag}{prefix}:");
        chat_lines.push(
            Line::raw(prefix_text)
                .style(Style::default().fg(role_color).add_modifier(Modifier::BOLD)),
        );
        // Render message content as Markdown with a 2-space indent.
        if !msg.content.is_empty() {
            let md_lines = render_markdown(&msg.content);
            for md_line in md_lines {
                let mut spans = vec![Span::raw("  ")];
                spans.extend(
                    md_line
                        .spans
                        .into_iter()
                        .map(|s| Span::styled(s.content.to_string(), s.style)),
                );
                let mut line = Line::from(spans);
                line.style = md_line.style;
                chat_lines.push(line);
            }
        }
        // Render tool call blocks with status indicators
        if let Some(tcs) = &msg.tool_calls {
            for tc in tcs {
                let is_pending = app.agent().status == AiStatus::AwaitingApproval
                    && app
                        .pending_agent_changes
                        .iter()
                        .any(|c| c.tool_call_id == tc.id);
                let has_tool_result = app.agent().messages.iter().any(|m| {
                    matches!(m.role, AiRole::Tool) && m.tool_call_id.as_deref() == Some(&tc.id)
                });
                let is_error = has_tool_result
                    && app.agent().messages.iter().any(|m| {
                        matches!(m.role, AiRole::Tool)
                            && m.tool_call_id.as_deref() == Some(&tc.id)
                            && m.content.starts_with("Error:")
                    });
                let (icon, status_color) = if is_error {
                    ("✗", Color::Red)
                } else if has_tool_result {
                    ("✓", Color::Green)
                } else if is_pending {
                    ("◆", Color::Yellow)
                } else {
                    ("⏳", Color::Yellow)
                };
                let duration_str = tc
                    .execution_duration_ms
                    .map(|ms| {
                        if ms >= 1000 {
                            format!("{:.1}s", ms as f64 / 1000.0)
                        } else {
                            format!("{ms}ms")
                        }
                    })
                    .unwrap_or_default();
                let tool_line = if duration_str.is_empty() {
                    format!("  🔧 {}  {icon}", tc.name)
                } else {
                    format!("  🔧 {}  {icon} {duration_str}", tc.name)
                };
                chat_lines.push(
                    Line::raw(tool_line).style(
                        Style::default()
                            .fg(status_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                );
            }
        }
        chat_lines.push(Line::raw(""));
    }

    // ── Inline diff for pending changes ────────────────────────
    if app.agent().status == AiStatus::AwaitingApproval && !app.pending_change_diffs.is_empty() {
        let total_diffs = app.pending_change_diffs.len();
        let title = if total_diffs > 1 {
            format!(" Pending Change (1 of {total_diffs}) ")
        } else {
            " Pending Change ".to_string()
        };
        chat_lines.push(Line::raw(""));
        chat_lines.push(
            Line::raw(format!("  🔧{title}")).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        );
        if let Some(diff) = app.pending_change_diffs.first() {
            let max_lines = chat_area
                .height
                .saturating_sub(chat_lines.len() as u16)
                .clamp(3, 20) as usize;
            for dl in diff.iter().take(max_lines) {
                let prefix = match dl.kind {
                    ChangeKind::Added => "+",
                    ChangeKind::Modified => "~",
                    ChangeKind::Deleted => "-",
                    ChangeKind::Unchanged => " ",
                };
                let color = match dl.kind {
                    ChangeKind::Added => Color::Green,
                    ChangeKind::Modified => Color::Yellow,
                    ChangeKind::Deleted => Color::Red,
                    ChangeKind::Unchanged => Color::DarkGray,
                };
                chat_lines.push(Line::styled(
                    format!("    {} {}", prefix, dl.text),
                    Style::default().fg(color),
                ));
            }
        }
        chat_lines.push(Line::raw(""));
    }

    // Show streaming partial response
    if !app.agent().partial_response.is_empty() {
        chat_lines.push(
            Line::raw("▷ 🥰:").style(
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        );
        let md_lines = render_markdown(&app.agent().partial_response);
        for md_line in md_lines {
            let mut spans = vec![Span::raw("  ")];
            spans.extend(
                md_line
                    .spans
                    .into_iter()
                    .map(|s| Span::styled(s.content.to_string(), s.style)),
            );
            let mut line = Line::from(spans);
            line.style = md_line.style;
            chat_lines.push(line);
        }
        let last_line = chat_lines.pop().unwrap_or(Line::raw(""));
        let mut spans: Vec<Span<'_>> = last_line.spans.into_iter().collect();
        spans.push(Span::styled(
            "▌",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::SLOW_BLINK),
        ));
        chat_lines.push(Line::from(spans));
        chat_lines.push(Line::raw(""));
    }

    // Slice chat history based on scroll offset
    let total_lines = chat_lines.len();
    let max_start = total_lines.saturating_sub(chat_area.height as usize);
    let start = max_start.saturating_sub(app.agent().scroll_offset.min(max_start));
    let end = (start + chat_area.height as usize).min(total_lines);
    let visible_lines: Vec<Line<'static>> = chat_lines[start..end].to_vec();

    frame.render_widget(
        Paragraph::new(Text::from(visible_lines))
            .wrap(ratatui::widgets::Wrap { trim: false })
            .style(Style::default()),
        chat_area,
    );

    // ── Status bar ────────────────────────────────────────────────
    let status_text: String = match app.agent().status {
        AiStatus::Waiting
            if app.agent().partial_response.is_empty() && app.agent().tool_status.is_some() =>
        {
            let spinner = spinner_frame();
            app.agent()
                .tool_status
                .clone()
                .unwrap_or_else(|| format!("{spinner} Teshi is thinking..."))
        }
        AiStatus::Waiting if app.agent().partial_response.is_empty() => {
            format!("{} Teshi is thinking...", spinner_frame())
        }
        AiStatus::AwaitingApproval => "◆ Waiting for approval — Y/N".into(),
        AiStatus::Error => {
            if app.status.starts_with("AI error:") {
                app.status.clone()
            } else {
                "AI error — check TESHI_LLM_API_KEY and your network connection.".to_string()
            }
        }
        _ => String::new(),
    };
    let model_label = app.active_model_label.as_deref().unwrap_or("");

    if status_area.width > 0 {
        let model_w = model_label.len() as u16;
        let [status_left, status_right] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(model_w)])
                .areas(status_area);

        if !status_text.is_empty() {
            let st_style = match app.agent().status {
                AiStatus::Waiting => Style::default().fg(Color::Yellow),
                AiStatus::AwaitingApproval => Style::default().fg(Color::Cyan),
                AiStatus::Error => Style::default().fg(Color::Red),
                _ => Style::default().fg(Color::DarkGray),
            };
            frame.render_widget(
                Paragraph::new(Text::from(Line::raw(status_text).style(st_style))),
                status_left,
            );
        }
        if !model_label.is_empty() {
            frame.render_widget(
                Paragraph::new(Text::from(
                    Line::raw(model_label).style(Style::default().fg(Color::DarkGray)),
                )),
                status_right,
            );
        }
    }

    // ── Input bar ───────────────────────────────────────────────────
    let input_border_style = if focused && app.ai_input_focused {
        Style::default()
            .fg(HIGHLIGHT_SELECTED_FG)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let label = token_usage_label(
        app.agent().total_input_tokens,
        app.agent().total_output_tokens,
    );
    let input_block = if label.is_empty() {
        Block::default()
            .borders(Borders::ALL)
            .border_style(input_border_style)
    } else {
        Block::default()
            .borders(Borders::ALL)
            .border_style(input_border_style)
            .title_top(Line::raw(label).alignment(Alignment::Right))
    };
    let input_inner = input_block.inner(input_area);
    frame.render_widget(input_block, input_area);

    let input_display: Text<'static> = if app.agent().input.is_empty() {
        if focused && app.ai_input_focused {
            Text::raw("Type your message...")
        } else {
            Text::raw("")
        }
    } else {
        let raw = app.agent().input.as_str();
        let lines: Vec<Line<'static>> = raw
            .lines()
            .map(|l| Line::from(Span::raw(l.to_string())))
            .collect();
        Text::from(lines)
    };
    frame.render_widget(
        Paragraph::new(input_display)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .style(match app.agent().status {
                AiStatus::Waiting => Style::default().fg(Color::DarkGray),
                _ => Style::default(),
            }),
        input_inner,
    );

    // Cursor in input, accounting for wrapping
    if focused && app.ai_input_focused && app.agent().status != AiStatus::Waiting {
        let (vis_row, vis_col) = visual_cursor_pos(
            &app.agent().input,
            app.agent().input_cursor,
            input_inner.width,
        );
        frame.set_cursor_position((input_inner.x + vis_col, input_inner.y + vis_row));
    }
}

/// Renders the editor panel showing the active feature file.
///
/// When `preview` is true (stage 2), the panel is read-only with no cursor. Otherwise (stage 3),
/// it shows the full interactive editor with cursor highlighting.
fn render_editor_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect, preview: bool) {
    let title_base = if preview {
        if app.preview_title.is_empty() {
            "Preview".to_string()
        } else {
            app.preview_title.clone()
        }
    } else {
        app.file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Editor".to_string())
    };
    let title = if preview {
        format!("{title_base} (preview)")
    } else {
        title_base
    };
    let editor_block = Block::default().borders(Borders::ALL).title(title);
    let editor_area = editor_block.inner(area);
    frame.render_widget(editor_block, area);
    frame.render_widget(Clear, editor_area);

    let visible_lines = editor_area.height as usize;
    let buffer = if preview {
        app.preview_buffer.as_ref().unwrap_or(&app.buffer)
    } else {
        &app.buffer
    };
    let cursor_row = if preview {
        app.preview_cursor_row
    } else {
        app.cursor_row
    };
    let scroll_row = if preview {
        app.preview_scroll_row
    } else {
        app.scroll_row
    };
    let mut visible_rows = if preview {
        (0..buffer.line_count()).collect::<Vec<_>>()
    } else {
        app.visible_editor_rows()
    };
    if visible_rows.is_empty() {
        visible_rows.push(0);
    }
    let cursor_idx = visible_rows
        .iter()
        .position(|&row| row == cursor_row)
        .or_else(|| visible_rows.iter().rposition(|&row| row <= cursor_row))
        .unwrap_or(0);
    let mut scroll_idx = visible_rows
        .iter()
        .position(|&row| row == scroll_row)
        .or_else(|| visible_rows.iter().position(|&row| row >= scroll_row))
        .unwrap_or(0);
    if !preview {
        if cursor_idx < scroll_idx {
            scroll_idx = cursor_idx;
        } else if cursor_idx >= scroll_idx.saturating_add(visible_lines) {
            scroll_idx = cursor_idx.saturating_sub(visible_lines.saturating_sub(1));
        }
    } else {
        scroll_idx = cursor_idx.saturating_sub(visible_lines / 2);
    }

    let mut lines = Vec::with_capacity(visible_lines);
    let preview_row_style = Style::default().bg(PREVIEW_CURSOR_BG).fg(PREVIEW_CURSOR_FG);
    let mut step_state = StepHighlightState::default();
    for &row in visible_rows.iter().take(scroll_idx) {
        if row >= buffer.line_count() {
            break;
        }
        let line = buffer.line(row);
        let _ = highlight_line_with_state(&line, &mut step_state, buffer.language());
    }
    for visible_idx in scroll_idx..scroll_idx.saturating_add(visible_lines) {
        let Some(&row) = visible_rows.get(visible_idx) else {
            let empty = pad_line_to_width(
                Line::raw(String::new()),
                editor_area.width,
                Style::default(),
            );
            lines.push(empty);
            continue;
        };
        let line = buffer.line(row);
        let mut display_line = line.clone();
        if !preview && let Some(step_count) = app.folded_step_count(row) {
            display_line.push_str(&format!("  [folded: {step_count} steps]"));
        }
        let (display_line, pad_offset, pad_start) =
            step_line_display(&display_line, step_state.in_doc_string, buffer.language());
        let display_len = display_line.chars().count();
        let mut styled =
            highlight_line_with_state(&display_line, &mut step_state, buffer.language());

        // When a scenario is focused, dim steps in non-focused scenarios
        if !preview && let Some(focus_row) = app.editor_focus_scenario_row {
            let header_row = crate::bdd_nav::scenario_header_for_row(buffer, row);
            // Dim the row if it belongs to a different (non-focused) scenario
            // AND is NOT itself the focused scenario header
            let is_different_scenario =
                header_row.is_some() && header_row != Some(focus_row) && header_row != Some(row);
            let is_not_focused_header = row != focus_row;
            if is_different_scenario && is_not_focused_header {
                styled = apply_line_background(
                    styled,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                );
            }
        }

        if row == cursor_row && !preview {
            let nav_cell_style = selected_style(true);
            let line_len = display_len;
            if app.is_editor_nav_mode() {
                let focus_patch = selected_style(true);
                let hl_range = nav_body_char_range_in_buffer(buffer, row, &line);
                if let Some(mut r) = hl_range
                    && r.start < r.end
                {
                    if pad_offset > 0 && r.start >= pad_start {
                        r.start += pad_offset;
                        r.end += pad_offset;
                    }
                    styled = apply_patch_to_char_range(styled, r, focus_patch);
                }
            } else if app.step_input_active || app.step_keyword_picker.is_some() {
                let cursor_col = if pad_offset > 0 && app.cursor_col >= pad_start {
                    app.cursor_col + pad_offset
                } else {
                    app.cursor_col
                };
                if line_len == 0 {
                    styled = Line::from(vec![Span::styled(" ", nav_cell_style)]);
                } else if cursor_col < line_len {
                    styled = apply_patch_to_char_range(
                        styled,
                        cursor_col..cursor_col.saturating_add(1),
                        nav_cell_style,
                    );
                } else {
                    let mut spans = styled.spans;
                    spans.push(Span::styled(" ", nav_cell_style));
                    styled = Line::from(spans);
                }
            }
        } else if row == cursor_row && preview {
            // Do not patch syntax-highlight spans: patching by char range can leave columns with
            // default colors between spans, which terminals show as a bright "hole" or bar.
            styled = if display_line.is_empty() {
                Line::from(vec![Span::styled(" ", preview_row_style)])
            } else {
                Line::from(Span::styled(display_line.to_string(), preview_row_style))
            };
        }

        // Apply mouse-drag selection highlight (column-accurate).
        if let (Some(anchor), Some(end)) = (app.selection_anchor, app.selection_end) {
            let sel_lo = anchor.0.min(end.0);
            let sel_hi = anchor.0.max(end.0);
            if !preview && row >= sel_lo && row <= sel_hi {
                let sel_style = Style::default().bg(SELECTION_BG).fg(SELECTION_FG);
                let mut range = if sel_lo == sel_hi {
                    // Single row: highlight the exact column range
                    let lo_col = anchor.1.min(end.1);
                    let hi_col = anchor.1.max(end.1).min(display_len);
                    lo_col..hi_col
                } else if row == sel_lo {
                    // First row: from the earlier column to end of line
                    let col = if anchor.0 < end.0 { anchor.1 } else { end.1 };
                    col..display_len
                } else if row == sel_hi {
                    // Last row: from start of line to the later column
                    let col = if anchor.0 > end.0 { anchor.1 } else { end.1 }.min(display_len);
                    0..col
                } else {
                    // Middle rows: full row
                    0..display_len
                };
                // Adjust for step-keyword padding, matching the cursor-row logic
                if pad_offset > 0 && range.start >= pad_start {
                    range.start = range.start.saturating_add(pad_offset);
                }
                if pad_offset > 0 && range.end > pad_start {
                    range.end = range.end.saturating_add(pad_offset);
                }
                if range.start < range.end {
                    styled = apply_patch_to_char_range(styled, range, sel_style);
                }
            }
        }

        let pad_trail = if preview && row == cursor_row && row < buffer.line_count() {
            preview_row_style
        } else {
            Style::default()
        };
        styled = truncate_line_to_cols(styled, editor_area.width);
        styled = pad_line_to_width(styled, editor_area.width, pad_trail);
        lines.push(styled);
    }

    // Stage-2 feature preview (right pane): paint each inner row in two passes. `set_line` alone
    // can leave columns past a truncated long line unchanged in edge cases; an explicit full-width
    // space fill first forces every cell in this frame (helps terminal diff + Windows hosts).
    let buf = frame.buffer_mut();
    if preview {
        for i in 0..visible_lines {
            let y = editor_area.y.saturating_add(i as u16);
            if y >= editor_area.bottom() {
                break;
            }
            let buffer_row = visible_rows
                .get(scroll_idx.saturating_add(i))
                .copied()
                .unwrap_or(usize::MAX);
            let row_fill = if buffer_row == cursor_row && buffer_row < buffer.line_count() {
                preview_row_style
            } else {
                Style::default()
            };
            buf.set_string(
                editor_area.x,
                y,
                " ".repeat(editor_area.width as usize),
                row_fill,
            );
        }
    }
    for (i, line) in lines.iter().enumerate() {
        let y = editor_area.y.saturating_add(i as u16);
        if y >= editor_area.bottom() {
            break;
        }
        buf.set_line(editor_area.x, y, line, editor_area.width);
    }
    if preview {
        app.preview_scroll_row = visible_rows.get(scroll_idx).copied().unwrap_or(0);
    } else {
        app.scroll_row = visible_rows.get(scroll_idx).copied().unwrap_or(0);
        app.editor_panel_rect = Some(editor_area);
    }
    if !preview {
        render_step_keyword_picker(frame, app, editor_area);
    }
}

/// Draws the step-keyword overlay when [`App::step_keyword_picker`] is active.
fn render_step_keyword_picker(frame: &mut Frame<'_>, app: &App, editor_area: Rect) {
    let Some(picker) = app.step_keyword_picker else {
        return;
    };

    const TITLE: &str = "Step keyword";
    let all_kw = app.buffer.language().all_step_keywords();
    let keywords: Vec<&str> = all_kw
        .iter()
        .filter(|kw| kw.as_str() != "*")
        .map(|s| s.as_str())
        .collect();
    let max_kw_ch = keywords
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(5);
    let inner_w_ch = max_kw_ch.max(TITLE.chars().count()).saturating_add(2);
    let list_w = (inner_w_ch as u16).saturating_add(2);
    let n_items = keywords.len();
    let list_h = (n_items as u16).saturating_add(2);

    let visible_lines = editor_area.height as usize;
    let row_in_view = picker.buffer_row.saturating_sub(app.scroll_row);
    let y_below = if row_in_view < visible_lines {
        editor_area.y + 1 + row_in_view as u16
    } else {
        editor_area
            .y
            .saturating_add(editor_area.height.saturating_sub(list_h))
    };
    let max_y = editor_area.y + editor_area.height;
    let mut y = y_below;
    if y.saturating_add(list_h) > max_y {
        y = max_y.saturating_sub(list_h);
    }
    y = y.max(editor_area.y);

    let h_avail = max_y.saturating_sub(y);
    let h = list_h.min(h_avail).max(3);
    let w = list_w.min(editor_area.width).max(3);
    let area = Rect::new(editor_area.x, y, w, h);

    frame.render_widget(Clear, area);

    let block = Block::default().borders(Borders::ALL).title(TITLE);
    let inner = block.inner(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let row_width = inner.width as usize;
    let max_rows = inner.height as usize;

    let selected_row_style = selected_style(true);
    let normal = Style::default();

    let mut lines: Vec<Line> = Vec::with_capacity(max_rows.min(n_items));
    for (i, kw) in keywords.iter().enumerate().take(max_rows) {
        let style = if i == picker.selected {
            selected_row_style
        } else {
            normal
        };
        let mut text = String::from(" ");
        text.push_str(kw);
        let used = UnicodeWidthStr::width(text.as_str());
        let pad = row_width.saturating_sub(used);
        text.push_str(&" ".repeat(pad));
        lines.push(Line::from(Span::styled(text, style)));
    }

    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Renders the reserved panel placeholder (stage 3, right side).
fn render_reserved_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Run Details")
        .style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let detail_key = (app.explore_selected_feature, app.explore_selected_scenario);
    let detail_content = if let Some(detail) = app.explore_case_details.get(&detail_key) {
        let scenario_name = app
            .project
            .features
            .get(app.explore_selected_feature)
            .and_then(|f| f.scenarios.get(app.explore_selected_scenario))
            .map(|s| s.name.as_str())
            .unwrap_or("-");
        truncate_lines(build_case_detail_lines(scenario_name, detail), inner.width)
    } else {
        truncate_lines(no_run_detail_lines(), inner.width)
    };
    let planned_style = Style::default().fg(Color::DarkGray);
    let mut content = vec![
        Line::styled(
            "Planned features:",
            planned_style.add_modifier(Modifier::BOLD),
        ),
        Line::styled("Step implementation code", planned_style),
        Line::styled("BDD runner", planned_style),
        Line::styled("Test results", planned_style),
        Line::raw(""),
    ];
    content.extend(detail_content);
    let content = truncate_lines(content, inner.width);
    frame.render_widget(Paragraph::new(Text::from(content)), inner);
}

fn build_case_detail_lines(scenario_name: &str, detail: &CaseDetail) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::styled(
        format!("Scenario: {scenario_name}"),
        Style::default().add_modifier(Modifier::BOLD),
    ));
    lines.push(Line::raw(""));
    lines.push(Line::raw(format!(
        "Status: {}",
        status_label(detail.status)
    )));
    lines.push(Line::raw(format!("Case: {}", detail.case_id)));
    let duration = detail
        .duration_ms
        .map(|ms| format!("{ms} ms"))
        .unwrap_or_else(|| "-".to_string());
    lines.push(Line::raw(format!("Duration: {duration}")));

    if let Some(message) = &detail.message {
        lines.push(Line::raw(""));
        lines.push(Line::raw("Message:"));
        lines.push(Line::raw(message.clone()));
    }

    if let Some(stack) = &detail.stack {
        lines.push(Line::raw(""));
        lines.push(Line::raw("Stack:"));
        for line in stack.lines() {
            lines.push(Line::raw(line.to_string()));
        }
    }

    if !detail.attachments.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::raw("Attachments:"));
        for att in &detail.attachments {
            lines.push(Line::raw(format!("- {}: {}", att.kind, att.path)));
        }
    }

    if !detail.logs.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::raw("Logs:"));
        for line in detail.logs.iter().take(20) {
            lines.push(Line::raw(line.clone()));
        }
    }

    lines
}

fn no_run_detail_lines() -> Vec<Line<'static>> {
    vec![
        Line::raw(""),
        Line::styled(
            "  No run details yet",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::raw("  Run [r] to execute scenarios."),
    ]
}

fn truncate_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let mut out_lines = Vec::with_capacity(lines.len());
    for line in lines {
        out_lines.push(truncate_line_to_cols(line, width));
    }
    out_lines
}

fn status_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Idle => "Idle",
        RunStatus::Running => "Running",
        RunStatus::Passed => "Passed",
        RunStatus::Failed => "Failed",
        RunStatus::Skipped => "Skipped",
    }
}

fn render_explore_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let feature_name = app
        .project
        .features
        .get(app.explore_selected_feature)
        .map(|f| feature_display_name(&f.file_path))
        .unwrap_or_else(|| "-".to_string());
    let scenario_name = app
        .project
        .features
        .get(app.explore_selected_feature)
        .and_then(|f| f.scenarios.get(app.explore_selected_scenario))
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "-".to_string());
    let left = format!("{feature_name}  {scenario_name}");

    let total = app
        .project
        .features
        .get(app.explore_selected_feature)
        .map(|f| f.scenarios.len())
        .unwrap_or(0);
    let right = if let Some(summary) = &app.explore_run_summary {
        format!(
            "{}/{} passed, {} failed",
            summary.passed, summary.total, summary.failed
        )
    } else {
        format!("0/{total} 通过")
    };

    let width = area.width as usize;
    let left_w = UnicodeWidthStr::width(left.as_str());
    let right_w = UnicodeWidthStr::width(right.as_str());

    let text = if right_w >= width {
        truncate_string_to_cols(right.as_str(), area.width)
    } else {
        let mut out = String::new();
        let mut left_trimmed = left;
        let avail_left = width.saturating_sub(right_w + 1);
        if left_w > avail_left {
            left_trimmed = truncate_string_to_cols(left_trimmed.as_str(), avail_left as u16);
        }
        let left_trimmed_w = UnicodeWidthStr::width(left_trimmed.as_str());
        let spaces = width.saturating_sub(left_trimmed_w + right_w);
        out.push_str(&left_trimmed);
        out.push_str(&" ".repeat(spaces));
        out.push_str(&right);
        out
    };

    frame.render_widget(Paragraph::new(text), area);
}

fn footer_pill(label: &'static str) -> Span<'static> {
    Span::styled(label, selected_style(false))
}

fn footer_hints(app: &App) -> Line<'static> {
    if app.is_editor_active() {
        return Line::from(vec![
            footer_pill(" Edit [Enter] "),
            Span::raw(" "),
            footer_pill(" Given/When/Then/And [Ctrl+g/w/t/a] "),
            Span::raw(" "),
            footer_pill(" Step [o/O] "),
            Span::raw(" "),
            footer_pill(" Delete [dd] "),
            Span::raw(" "),
            footer_pill(" Copy/Paste [yy/p] "),
            Span::raw(" "),
            footer_pill(" MoveStep [Ctrl+j/k] "),
            Span::raw(" "),
            footer_pill(" Fold [Space] "),
            Span::raw(" "),
            footer_pill(" FoldAll [Ctrl+Space] "),
            Span::raw(" "),
            footer_pill(" Save [Ctrl+s] "),
        ]);
    }
    match (app.active_tab, app.view_stage) {
        (MainTab::MindMap, _)
            if app.mindmap_focus == MindMapFocus::AiPanel && app.ai_input_focused =>
        {
            // AI chat input focused — show AI-style hints
            Line::from(vec![
                footer_pill(" Send [Enter] "),
                Span::raw(" "),
                footer_pill(" Back/Browse [Esc] "),
                Span::raw(" "),
                footer_pill(" Toggle Panel [p] "),
                Span::raw(" "),
                footer_pill(" Quit [q] "),
            ])
        }
        (MainTab::MindMap, _) if app.mindmap_focus == MindMapFocus::AiPanel => {
            // AI panel focused but input not focused
            Line::from(vec![
                footer_pill(" Focus Input [Enter] "),
                Span::raw(" "),
                footer_pill(" Scroll [Alt+↑↓] "),
                Span::raw(" "),
                footer_pill(" Return Tree [Esc] "),
                Span::raw(" "),
                footer_pill(" Toggle Panel [p] "),
                Span::raw(" "),
                footer_pill(" Quit [q] "),
            ])
        }
        (MainTab::MindMap, _) => {
            // Tree focused
            let mut hints = vec![
                footer_pill(" Navigate [↑↓] "),
                Span::raw(" "),
                footer_pill(" Toggle [Space] "),
                Span::raw(" "),
                footer_pill(" Expand [→] "),
                Span::raw(" "),
                footer_pill(" Collapse [←] "),
            ];
            if app.mindmap_ai_panel_visible {
                hints.push(Span::raw(" "));
                hints.push(footer_pill(" Focus Panel [Enter] "));
                hints.push(Span::raw(" "));
                hints.push(footer_pill(" Toggle Panel [p] "));
            } else {
                hints.push(Span::raw(" "));
                hints.push(footer_pill(" Show Panel [p] "));
            }
            hints.push(Span::raw(" "));
            hints.push(footer_pill(" Quit [q] "));
            Line::from(hints)
        }
        (MainTab::Explore, _) => Line::from(vec![
            footer_pill(" Focus [Tab/←→] "),
            Span::raw(" "),
            footer_pill(" Navigate [↑↓] "),
            Span::raw(" "),
            footer_pill(" Edit [e/→] "),
            Span::raw(" "),
            footer_pill(" Detail [Enter] "),
            Span::raw(" "),
            footer_pill(" Run [r] "),
            Span::raw(" "),
            footer_pill(" AI [a] "),
            Span::raw(" "),
            footer_pill(" Quit [q] "),
        ]),
        (MainTab::Ai, _) => Line::from(vec![
            footer_pill(" Model [m] "),
            Span::raw(" "),
            footer_pill(" Quit [q] "),
        ]),
    }
}

/// Renders the auth management overlay panel over the entire screen.
fn render_auth_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let panel_w = area.width.min(60);
    let panel_h = area.height.min(20);
    let panel_x = (area.width.saturating_sub(panel_w)) / 2;
    let panel_y = (area.height.saturating_sub(panel_h)) / 2;
    let panel_area = Rect::new(panel_x, panel_y, panel_w, panel_h);

    frame.render_widget(Clear, panel_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Auth Management ")
        .style(Style::default().fg(Color::White));
    let inner = block.inner(panel_area);
    frame.render_widget(block, panel_area);

    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::raw(""));

    match app.status.as_str() {
        s if s.starts_with("Auth: add provider") => {
            lines.push(Line::styled(
                "Add Provider",
                Style::default().add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(
                "Use 'teshi auth login' from the CLI to add a provider API key.",
            ));
        }
        s if s.starts_with("Auth: remove provider") => {
            let provider = s
                .strip_prefix("Auth: remove provider '")
                .and_then(|r| r.strip_suffix('\''))
                .unwrap_or("?");
            lines.push(Line::styled(
                format!("Remove Provider: {}", provider),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(
                "Use 'teshi auth remove <provider>' from the CLI to remove.",
            ));
        }
        _ => {
            // Default: show provider overview
            lines.push(Line::styled(
                "Configured Providers",
                Style::default().add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(""));

            if app.config.providers.is_empty() {
                lines.push(Line::raw("  No providers configured."));
            } else {
                for (name, provider) in &app.config.providers {
                    let model = provider.model.as_deref().unwrap_or("-");
                    let has_key = provider
                        .api_key
                        .as_ref()
                        .filter(|k| !k.is_empty())
                        .is_some();
                    let key_status = if has_key { "✓" } else { "✗" };
                    let key_color = if has_key { Color::Green } else { Color::Red };
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            format!("{:<14}", name),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(" key: {}", key_status),
                            Style::default().fg(key_color),
                        ),
                        Span::raw(format!("  model: {}", model)),
                    ]));
                }
            }

            if let Some(ref default) = app.config.default_provider {
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::raw("  Default: "),
                    Span::styled(default.as_str(), Style::default().fg(Color::Cyan)),
                ]));
            }
        }
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Esc to close",
        Style::default().fg(Color::DarkGray),
    ));

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Renders the model profile management overlay panel.
fn render_model_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let panel_w = area.width.min(60);
    let panel_h = area.height.min(22);
    let panel_x = (area.width.saturating_sub(panel_w)) / 2;
    let panel_y = (area.height.saturating_sub(panel_h)) / 2;
    let panel_area = Rect::new(panel_x, panel_y, panel_w, panel_h);

    frame.render_widget(Clear, panel_area);

    use crate::app::ModelPanelMode;

    match app.model_panel_mode {
        ModelPanelMode::List => render_model_list(frame, app, panel_area),
        ModelPanelMode::Adding => render_model_form(frame, app, " Add Model Profile ", panel_area),
        ModelPanelMode::Editing => {
            render_model_form(frame, app, " Edit Model Profile ", panel_area)
        }
    }
}

/// Renders the list of model profiles.
fn render_model_list(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Model Profiles ")
        .style(Style::default().fg(Color::White));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::raw(""));

    if app.model_profiles.is_empty() {
        lines.push(Line::from(vec![Span::raw("  No model profiles found.")]));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![Span::styled(
            "  Press 'a' to add one, or place a TOML file in",
            Style::default().fg(Color::DarkGray),
        )]));
        lines.push(Line::from(vec![Span::styled(
            "  ~/.config/teshi/models/",
            Style::default().fg(Color::DarkGray),
        )]));
    } else {
        for (i, profile) in app.model_profiles.iter().enumerate() {
            let is_active = app.model_active_id.as_deref() == Some(&profile.id);
            let prefix = if i == app.model_panel_selection {
                " ▸ "
            } else {
                "   "
            };
            let active_mark = if is_active { "  [ACTIVE]" } else { "" };

            let style = if i == app.model_panel_selection {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if is_active {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };

            lines.push(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(format!("{:<20}", profile.name), style),
                Span::raw("  "),
                Span::styled(
                    format!("{} | {}", profile.provider, profile.model),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(active_mark, Style::default().fg(Color::Green)),
            ]));
        }
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "a add · e edit · d delete · ↑↓ select · Enter activate · Esc close",
        Style::default().fg(Color::DarkGray),
    ));

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Renders the model profile form (add or edit).
fn render_model_form(frame: &mut Frame<'_>, app: &App, title: &str, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().fg(Color::White));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // 7 form fields
    let fields: [(&str, &str); 7] = [
        ("Name", &app.model_form_name),
        ("Provider", &app.model_form_provider),
        ("Model", &app.model_form_model),
        ("Base URL", &app.model_form_base_url),
        ("API Key", &app.model_form_api_key),
        ("Max Tokens", &app.model_form_max_tokens),
        ("Temperature", &app.model_form_temperature),
    ];

    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::raw(""));

    for (i, (label, value)) in fields.iter().enumerate() {
        let focused = i == app.model_form_focus;
        let indicator = if focused { " ▸ " } else { "   " };
        let label_style = if focused {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };
        let val_style = if focused {
            Style::default().bg(Color::DarkGray).fg(Color::White)
        } else {
            Style::default().fg(Color::White)
        };
        let display_val = if value.is_empty() {
            if focused {
                "(type here...)".to_string()
            } else {
                "(empty)".to_string()
            }
        } else {
            // Mask API key when not focused
            if i == 4 && !focused && value.len() > 8 {
                format!("{}...{}", &value[..4], &value[value.len() - 4..])
            } else {
                value.to_string()
            }
        };

        lines.push(Line::from(vec![
            Span::styled(indicator, label_style),
            Span::styled(format!("{:<12}", label), label_style),
            Span::raw(" "),
            Span::styled(display_val, val_style),
        ]));
    }

    let footer = Line::styled(
        "Save [Enter]  Cancel [Esc]",
        Style::default().fg(Color::DarkGray),
    )
    .alignment(Alignment::Center);
    lines.push(footer);

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Renders the session browser overlay panel.
fn render_session_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let panel_w = area.width.min(60);
    let panel_h = area.height.min(20);
    let x = area.x + (area.width.saturating_sub(panel_w)) / 2;
    let y = area.y + (area.height.saturating_sub(panel_h)) / 2;
    let panel_area = Rect::new(x, y, panel_w, panel_h);

    // Dim background
    frame.render_widget(Clear, panel_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Sessions ")
        .style(Style::default());
    let inner = block.inner(panel_area);
    frame.render_widget(block, panel_area);

    if inner.width < 10 || inner.height < 2 {
        return;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();

    if app.session_list.is_empty() {
        lines.push(Line::raw("  No saved sessions."));
        lines.push(Line::raw(""));
        lines.push(Line::raw(
            "  Type /new to start a session that will be saved.",
        ));
    } else {
        for (i, session) in app.session_list.iter().enumerate() {
            let is_selected = i == app.session_panel_selection;
            let prefix = if is_selected { " ▸ " } else { "   " };
            // Show created_at as readable date
            let date = session.created_at.as_str();
            let label = format!("{}{} msgs — {}", prefix, session.message_count, date);
            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(Line::styled(label, style));
            if let Some(ref ml) = session.model_label {
                lines.push(
                    Line::raw(format!("    Model: {ml}"))
                        .style(Style::default().fg(Color::DarkGray)),
                );
            }
        }
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled(
        " ↑↓ select · Enter load · d delete · Esc close ",
        Style::default().fg(Color::DarkGray),
    ));

    let visible: Vec<Line<'static>> = lines.into_iter().take(inner.height as usize).collect();
    frame.render_widget(Paragraph::new(Text::from(visible)), inner);
}

/// Render the quit confirmation popup panel.
fn render_quit_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let panel_w = area.width.min(40);
    let panel_h = 7;
    let panel_x = (area.width.saturating_sub(panel_w)) / 2;
    let panel_y = (area.height.saturating_sub(panel_h)) / 2;
    let panel_area = Rect::new(panel_x, panel_y, panel_w, panel_h);

    frame.render_widget(Clear, panel_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Quit Teshi? ")
        .border_style(Style::default().fg(Color::Red));
    let inner = block.inner(panel_area);
    frame.render_widget(block, panel_area);

    let inner_w = inner.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Centered message
    let msg = "You have unsaved changes, quit anyway?";
    let msg_pad = (inner_w.saturating_sub(msg.len())) / 2;
    lines.push(Line::styled(
        format!("{}{}", " ".repeat(msg_pad), msg),
        Style::default().fg(Color::White),
    ));
    lines.push(Line::raw(""));

    // Symmetric button row: "  [Yes]    [No]  " (20 chars)
    let btn_row = inner.y + 2;
    let btn_sep = "    "; // 4 chars gap
    // Total: "  " + btn_text + btn_sep + btn_text + "  " = 2+6+4+6+2 = 20
    let btn_line_total = 20usize;
    let btn_left_pad = (inner_w.saturating_sub(btn_line_total)) / 2;
    let col_yes_start = inner.x + btn_left_pad as u16 + 2;
    let col_no_start = col_yes_start + 6 + 4; // after Yes + separator

    // Check hover states and keyboard focus
    let yes_hovered = app
        .mouse_position
        .is_some_and(|(mx, my)| my == btn_row && mx >= col_yes_start && mx < col_yes_start + 6);
    let no_hovered = app
        .mouse_position
        .is_some_and(|(mx, my)| my == btn_row && mx >= col_no_start && mx < col_no_start + 6);

    // Keyboard selection always shows a focused button (mouse hover overrides)
    let yes_focused = app.quit_panel_selection == 0;
    let no_focused = app.quit_panel_selection == 1;

    // Register clickable regions
    app.clickable_regions.push(ClickableRegion::QuitConfirmYes {
        row_y: btn_row,
        col_x: col_yes_start,
        col_right: col_yes_start + 6,
    });
    app.clickable_regions.push(ClickableRegion::QuitConfirmNo {
        row_y: btn_row,
        col_x: col_no_start,
        col_right: col_no_start + 6,
    });

    let yes_style = if yes_hovered {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else if yes_focused {
        Style::default()
            .fg(Color::Yellow)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    };
    let no_style = if no_hovered {
        Style::default()
            .fg(Color::Black)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else if no_focused {
        Style::default()
            .fg(Color::Yellow)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    };

    let btn_line = Line::from(vec![
        Span::raw(" ".repeat(btn_left_pad)),
        Span::raw("  "),
        Span::styled(" [Yes] ", yes_style),
        Span::raw(btn_sep),
        Span::styled(" [No] ", no_style),
        Span::raw("  "),
    ]);
    lines.push(btn_line);

    let visible: Vec<Line<'static>> = lines.into_iter().take(inner.height as usize).collect();
    frame.render_widget(Paragraph::new(Text::from(visible)), inner);
}

/// Render the approval mode selection popup panel.
fn render_approval_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    use crate::agent::approval::ApprovalMode;

    let modes = ApprovalMode::ALL;
    let panel_w = area.width.min(56);
    // Layout: top border + 3 items × (title + desc) + separator line + bottom border
    let item_rows: u16 = (modes.len() * 2 + 1) as u16; // +1 for separator
    let panel_h = item_rows + 3; // +3 for borders + padding
    let panel_x = area.x + (area.width.saturating_sub(panel_w)) / 2;
    let panel_y = area.y + (area.height.saturating_sub(panel_h)) / 2;
    let panel_area = Rect::new(panel_x, panel_y, panel_w, panel_h);

    frame.render_widget(Clear, panel_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Approval Mode ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(panel_area);
    frame.render_widget(block, panel_area);

    // Track which rows are rendered so we can register clickable regions
    let mut row_offset = inner.y;
    let mut lines: Vec<Line<'static>> = Vec::new();

    for (i, mode) in modes.iter().enumerate() {
        let is_current = *mode == app.approval_mode;
        let is_selected = i == app.approval_panel_selection;
        let label = mode.display_name();
        let desc = mode.description();

        // ── Title line ──
        let (main_fg, desc_fg, marker) = if is_selected {
            // Selected item: always yellow with arrow, regardless of current
            (Color::Yellow, Color::DarkGray, "\u{25b6}")
        } else if is_current {
            // Current but not selected: dimmed with circle
            (Color::DarkGray, Color::DarkGray, "\u{25cb}")
        } else {
            (Color::White, Color::Gray, "  ")
        };
        let title = format!(" {} {}", marker, label);
        let mut title_style =
            Style::default()
                .fg(main_fg)
                .add_modifier(if is_selected || is_current {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                });
        // Hover effect: reverse video to indicate clickability
        let is_hovered = app
            .mouse_position
            .is_some_and(|(mx, my)| my == row_offset && mx >= inner.x && mx < inner.right());
        if is_hovered {
            title_style = title_style.add_modifier(Modifier::REVERSED);
        }
        lines.push(Line::styled(title, title_style));

        // Register clickable region for this option's title row
        app.clickable_regions.push(ClickableRegion::ApprovalOption {
            option_idx: i,
            row_y: row_offset,
            col_x: inner.x,
            col_right: inner.right(),
        });
        row_offset += 1;

        // ── Description line ──
        let desc_text = format!("   {}", desc);
        lines.push(Line::styled(desc_text, Style::default().fg(desc_fg)));
        row_offset += 1;

        // ── Separator ──
        if i < modes.len() - 1 {
            lines.push(Line::raw(""));
            row_offset += 1;
        }
    }

    let visible: Vec<Line<'static>> = lines.into_iter().take(inner.height as usize).collect();
    frame.render_widget(Paragraph::new(Text::from(visible)), inner);
}

/// Render the agent profile selection popup panel.
fn render_agent_profile_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    use crate::app::AgentPanelMode;

    match app.agent_panel_mode {
        AgentPanelMode::List => render_agent_profile_list(frame, app, area),
        AgentPanelMode::Adding => {
            render_agent_profile_form(frame, app, " Add Agent Profile ", area)
        }
        AgentPanelMode::Editing => {
            render_agent_profile_form(frame, app, " Edit Agent Profile ", area)
        }
    }
}

fn render_agent_profile_list(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let profiles = &app.agent_profiles;
    let panel_w = area.width.min(56);
    let item_rows: u16 = profiles.len() as u16;
    let panel_h = (item_rows * 2 + 4).min(area.height.saturating_sub(4));
    let panel_x = area.x + (area.width.saturating_sub(panel_w)) / 2;
    let panel_y = area.y + (area.height.saturating_sub(panel_h)) / 2;
    let panel_area = Rect::new(panel_x, panel_y, panel_w, panel_h);

    frame.render_widget(Clear, panel_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Agent Profile ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(panel_area);
    frame.render_widget(block, panel_area);

    let mut lines: Vec<Line<'static>> = Vec::new();

    for (i, profile) in profiles.iter().enumerate() {
        // Determine if this profile is active for the current agent
        let is_active = app
            .agent()
            .profile_id
            .as_deref()
            .map_or(profile.id == "default", |id| id == profile.id);
        let is_selected = i == app.agent_profile_panel_selection;

        // ── Title line ──
        let (main_fg, marker) = if is_selected {
            (Color::Yellow, "\u{25b6}")
        } else if is_active {
            (Color::Green, "\u{25cb}")
        } else {
            (Color::White, "  ")
        };
        let active_tag = if is_active { " [ACTIVE]" } else { "" };
        let title = format!(" {} {}{}", marker, profile.name, active_tag);
        lines.push(Line::styled(
            title,
            Style::default()
                .fg(main_fg)
                .add_modifier(if is_selected || is_active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));

        // ── Description line ──
        lines.push(Line::styled(
            format!("   {}", profile.description),
            Style::default().fg(Color::DarkGray),
        ));

        // ── Separator ──
        if i < profiles.len() - 1 {
            lines.push(Line::raw(""));
        }
    }

    // Navigation hint
    lines.push(Line::styled(
        "a add · e edit · d delete · ↑↓ select · Enter apply · Esc close",
        Style::default().fg(Color::DarkGray),
    ));

    let visible: Vec<Line<'static>> = lines.into_iter().take(inner.height as usize).collect();
    frame.render_widget(Paragraph::new(Text::from(visible)), inner);
}

fn render_agent_profile_form(frame: &mut Frame<'_>, app: &App, title: &str, area: Rect) {
    let panel_w = area.width.min(72);
    let panel_h = 22u16.min(area.height.saturating_sub(4));
    let panel_x = area.x + (area.width.saturating_sub(panel_w)) / 2;
    let panel_y = area.y + (area.height.saturating_sub(panel_h)) / 2;
    let panel_area = Rect::new(panel_x, panel_y, panel_w, panel_h);

    frame.render_widget(Clear, panel_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(panel_area);
    frame.render_widget(block, panel_area);

    // ── Tab bar ──
    let tabs = ["Basic", "Instructions", "Tools", "Skills", "Model"];
    let tab_line: Vec<Span<'_>> = tabs
        .iter()
        .enumerate()
        .flat_map(|(i, name)| {
            let is_active = i == app.agent_config_tab;
            let style = if is_active {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
            };
            let mut parts = vec![Span::styled(format!(" {} ", name), style)];
            if i < tabs.len() - 1 {
                parts.push(Span::raw("│"));
            }
            parts
        })
        .collect();

    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::from(tab_line));
    lines.push(Line::styled(
        "─".repeat(inner.width as usize),
        Style::default().fg(Color::DarkGray),
    ));

    // ── Tab content ──
    match app.agent_config_tab {
        0 => {
            // Basic — Name + Description
            render_tab_field(&mut lines, app, 0, "Name", &app.agent_form_name);
            render_tab_field(
                &mut lines,
                app,
                1,
                "Description",
                &app.agent_form_description,
            );
        }
        1 => {
            // Instructions — system prompt
            let focused = app.agent_form_focus == 2;
            let indicator = if focused { " ▸ " } else { "   " };
            let label_style = if focused {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
            };
            let val_style = if focused {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled(indicator, label_style),
                Span::styled("Instructions", label_style),
            ]));
            let instr_text = if app.agent_form_instructions.is_empty() {
                if focused { "(type here...)" } else { "(empty)" }
            } else {
                &app.agent_form_instructions
            };
            // Show first few lines of the instructions
            for line_text in instr_text.lines().take(6) {
                lines.push(Line::styled(format!("   {}", line_text), val_style));
            }
            if instr_text.lines().count() > 6 {
                lines.push(Line::styled("   ...", val_style));
            }
            lines.push(Line::styled(
                "   (║ text will wrap in the terminal)",
                Style::default().fg(Color::DarkGray),
            ));
        }
        2 => {
            // Tools — comma-separated
            render_tab_field(&mut lines, app, 3, "Tools (csv)", &app.agent_form_tools_str);
            lines.push(Line::styled(
                "   Available: get_project_info, get_feature_content, insert_scenario, ...",
                Style::default().fg(Color::DarkGray),
            ));
        }
        3 => {
            // Skills — comma-separated directories
            render_tab_field(
                &mut lines,
                app,
                4,
                "Skills dirs (csv)",
                &app.agent_form_skills_str,
            );
            lines.push(Line::styled(
                "   Paths relative to project root, e.g. .teshi/skills",
                Style::default().fg(Color::DarkGray),
            ));
        }
        4 => {
            // Model — model_ref
            render_tab_field(&mut lines, app, 5, "Model ref", &app.agent_form_model_ref);
            lines.push(Line::styled(
                "   Leave empty to use the active model profile",
                Style::default().fg(Color::DarkGray),
            ));
        }
        _ => {}
    }

    // ── Footer ──
    lines.push(Line::raw(""));
    let footer = Line::styled(
        "← → tabs · Tab/↑↓ field · Enter save · Esc cancel",
        Style::default().fg(Color::DarkGray),
    )
    .alignment(Alignment::Center);
    lines.push(footer);

    let visible: Vec<Line<'static>> = lines.into_iter().take(inner.height as usize).collect();
    frame.render_widget(Paragraph::new(Text::from(visible)), inner);
}

/// Helper to render a single form field row (label + value + focus highlight).
fn render_tab_field(
    lines: &mut Vec<Line<'static>>,
    app: &App,
    focus_idx: usize,
    label: &str,
    value: &str,
) {
    let focused = app.agent_form_focus == focus_idx;
    let indicator = if focused { " ▸ " } else { "   " };
    let label_style = if focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Cyan)
    };
    let val_style = if focused {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    } else {
        Style::default().fg(Color::White)
    };
    let display_val = if value.is_empty() {
        if focused {
            "(type here...)".to_string()
        } else {
            "(empty)".to_string()
        }
    } else {
        value.to_string()
    };

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(indicator, label_style),
        Span::styled(format!("{:<18}", label), label_style),
        Span::raw(" "),
        Span::styled(display_val, val_style),
    ]));
}

#[cfg(test)]
mod truncate_tests {
    use std::path::PathBuf;

    use super::{Line, Span, explore_scenarios_title, truncate_line_to_cols};
    use crate::app::App;
    use crate::gherkin;

    #[test]
    fn truncate_line_to_cols_limits_display_width() {
        let line = Line::from(Span::raw("a".repeat(100)));
        assert!(line.width() > 68);
        let out = truncate_line_to_cols(line, 68);
        assert!(out.width() <= 68);
    }

    #[test]
    fn test_explore_scenarios_title_shows_zero_when_no_feature_selected() {
        let app = App::from_args().expect("app init should work");
        assert_eq!(explore_scenarios_title(&app), "Scenarios (0)");
    }

    #[test]
    fn test_explore_scenarios_title_shows_selected_feature_scenario_count() {
        let mut app = App::from_args().expect("app init should work");
        let feature = gherkin::parse_feature(
            "Feature: A\n  Scenario: S1\n    Given a\n  Scenario: S2\n    Given b\n",
            PathBuf::from("a.feature"),
        );
        app.project.features = vec![feature];
        app.explore_selected_feature = 0;
        assert_eq!(explore_scenarios_title(&app), "Scenarios (2)");
    }
}
