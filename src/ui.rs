use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs};
use tui_tree_widget::Tree;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{
    App, CaseDetail, ColumnFocus, MainTab, MindMapFocus, RunStatus, STEP_KEYWORDS_CYCLE,
};
use crate::bdd_nav::nav_body_char_range_in_buffer;
use crate::highlight::{KeywordSet, StepHighlightState, highlight_line_with_state};

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
const HIGHLIGHT_FOCUSED_FG: Color = Color::White;
const HIGHLIGHT_UNFOCUSED_FG: Color = Color::Black;

fn selected_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .bg(EXPLORE_SELECTED_FOCUSED_BG)
            .fg(HIGHLIGHT_FOCUSED_FG)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .bg(EXPLORE_SELECTED_UNFOCUSED_BG)
            .fg(HIGHLIGHT_UNFOCUSED_FG)
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

fn step_line_display(line: &str, in_doc_string: bool) -> (String, usize, usize) {
    if in_doc_string {
        return (line.to_string(), 0, 0);
    }
    let trimmed = line.trim_start();
    let leading = line.len().saturating_sub(trimmed.len());
    let Some(keyword) = STEP_KEYWORDS_CYCLE
        .iter()
        .find(|kw| trimmed.starts_with(**kw))
    else {
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
        Line::from(" Help [4] "),
    ])
    .select(match app.active_tab {
        MainTab::Explore => 0,
        MainTab::MindMap => 1,
        MainTab::Ai => 2,
        MainTab::Help => 3,
    })
    .style(Style::default().fg(Color::DarkGray))
    .highlight_style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )
    .divider(" ");
    frame.render_widget(top_tabs, chunks[0]);

    let divider_w = chunks[1].width as usize;
    let divider_line = "─".repeat(divider_w.max(1));
    frame.render_widget(
        Paragraph::new(divider_line).style(Style::default().fg(Color::DarkGray)),
        chunks[1],
    );

    render_main_panel(frame, app, chunks[2]);

    // Agent change confirmation prompt takes top priority
    if app.has_agent_change_prompt() {
        let prompt = Line::from(vec![
            Span::styled(
                "AI wants to modify a file",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" — "),
            Span::styled(
                "[Y]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" accept  "),
            Span::styled(
                "[N]",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" reject  "),
            Span::styled("[Esc]", Style::default().fg(Color::DarkGray)),
            Span::raw(" reject"),
        ]);
        frame.render_widget(Paragraph::new(prompt), chunks[3]);
    } else if let Some(ref msg) = app.status_message {
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
}

fn render_main_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    match app.active_tab {
        MainTab::MindMap => render_mindmap_panel(frame, app, area),
        MainTab::Explore => render_explore_panel(frame, app, area),
        MainTab::Ai => render_ai_panel(frame, app, area),
        MainTab::Help => {
            let block = Block::default().borders(Borders::ALL).title("Help");
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let help = vec![
                Line::raw("Tabs: Explore [1], MindMap [2], Help [3], AI [4]"),
                Line::raw(""),
                Line::raw("── MindMap (Tree + AI Preview) ──"),
                Line::raw("↑↓ navigate tree   Space toggle   ←→ collapse/expand"),
                Line::raw("Enter focus AI preview   Esc/← return to tree"),
                Line::raw("Ctrl+\\ toggle AI preview panel   a send to AI chat"),
                Line::raw("Home/End first/last node"),
                Line::raw(""),
                Line::raw("── Explore (Three Columns) ──"),
                Line::raw("Tab/Shift+Tab/←→ switch column   ↑↓ navigate"),
                Line::raw("→ on Step or e edit   Enter details   Esc/← exit edit"),
                Line::raw("r run   a AI"),
                Line::raw(""),
                Line::raw("── Editor (Edit Mode) ──"),
                Line::raw("hjkl / arrows navigate   Enter edit"),
                Line::raw("o/O add step   dd delete   yy copy   p paste"),
                Line::raw("Ctrl+g/w/t/a switch keyword   Ctrl+j/k move step"),
                Line::raw("Space fold scenario   Ctrl+Space fold all"),
                Line::raw("Ctrl+r run   Ctrl+s save   Ctrl+/ undo   Ctrl+y redo"),
                Line::raw(""),
                Line::raw("── AI Tab ──"),
                Line::raw("Type a message and press Enter to chat with the LLM."),
                Line::raw("Set TESHI_LLM_API_KEY in your environment to enable."),
                Line::raw(""),
                Line::raw("Global: s save, q quit (dirty needs confirmation)"),
            ];
            frame.render_widget(Paragraph::new(Text::from(help)), inner);
        }
    }
}

/// Renders the AI chat panel.
fn render_ai_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    use crate::app::{AiRole, AiStatus};

    if area.width < 10 || area.height < 3 {
        return;
    }

    let block = Block::default().borders(Borders::ALL).title("AI Chat");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Layout: chat history (top) + input bar (bottom)
    let input_height: u16 = 3;
    let chat_height = inner.height.saturating_sub(input_height);
    let chat_area = Rect::new(inner.x, inner.y, inner.width, chat_height);
    let input_area = Rect::new(inner.x, inner.y + chat_height, inner.width, input_height);

    // ── Chat history ────────────────────────────────────────────────
    let status_style = match app.ai_status {
        AiStatus::Waiting => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::SLOW_BLINK),
        AiStatus::Error => Style::default().fg(Color::Red),
        AiStatus::Idle => Style::default().fg(Color::DarkGray),
    };
    let status_line = match app.ai_status {
        AiStatus::Waiting if app.ai_partial_response.is_empty() && app.ai_tool_status.is_some() => {
            let text = app
                .ai_tool_status
                .clone()
                .unwrap_or_else(|| "AI is thinking...".into());
            Line::raw(text)
        }
        AiStatus::Waiting if app.ai_partial_response.is_empty() => Line::raw("AI is thinking..."),
        AiStatus::Waiting => Line::raw(""),
        AiStatus::Error => {
            let err_text = if app.status.starts_with("AI error:") {
                app.status.clone()
            } else {
                "AI error — check TESHI_LLM_API_KEY and your network connection.".to_string()
            };
            Line::raw(err_text)
        }
        AiStatus::Idle => Line::raw(""),
    };

    let mut chat_lines: Vec<Line<'static>> = Vec::new();

    // Add a greeting if no messages
    if app.ai_messages.is_empty() {
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

    for msg in &app.ai_messages {
        // Skip internal tool messages — they are not user-visible.
        if matches!(msg.role, AiRole::Tool) {
            continue;
        }
        let prefix = match msg.role {
            AiRole::User => "▶ You",
            AiRole::Assistant => "✦ AI",
            AiRole::Tool => unreachable!(),
        };
        let role_color = match msg.role {
            AiRole::User => Color::Cyan,
            AiRole::Assistant => Color::Green,
            AiRole::Tool => unreachable!(),
        };
        // Show source tag for MindMap-initiated messages
        let source_tag = msg
            .source
            .as_ref()
            .map(|s| format!("[{s}] "))
            .unwrap_or_default();
        // Show tool call indicator if present
        let tool_note = msg.tool_calls.as_ref().map(|tcs| {
            let names: Vec<&str> = tcs.iter().map(|tc| tc.name.as_str()).collect();
            format!(" [called: {}]", names.join(", "))
        });
        let prefix_text = if let Some(note) = tool_note {
            format!("{source_tag}{prefix}:{note}")
        } else {
            format!("{source_tag}{prefix}:")
        };
        chat_lines.push(
            Line::raw(prefix_text)
                .style(Style::default().fg(role_color).add_modifier(Modifier::BOLD)),
        );
        // Wrap long messages
        for line_text in msg.content.lines() {
            chat_lines.push(Line::raw(format!("  {line_text}")));
        }
        chat_lines.push(Line::raw(""));
    }

    // Show streaming partial response as a live assistant message
    if !app.ai_partial_response.is_empty() {
        chat_lines.push(
            Line::raw("✦ AI:").style(
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        );
        for line_text in app.ai_partial_response.lines() {
            chat_lines.push(Line::raw(format!("  {line_text}")));
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

    // Add the status line (only non-empty strings)
    let status_text = status_line
        .spans
        .iter()
        .fold(String::new(), |acc, s| acc + &s.to_string());
    if !status_text.trim().is_empty() {
        chat_lines.push(status_line.style(status_style));
    }

    // Slice chat history based on scroll offset (0 = show bottom)
    let total_lines = chat_lines.len();
    let max_start = total_lines.saturating_sub(chat_height as usize);
    let start = max_start.saturating_sub(app.ai_scroll_offset.min(max_start));
    let end = (start + chat_height as usize).min(total_lines);
    let visible_lines: Vec<Line<'static>> = chat_lines[start..end].to_vec();

    frame.render_widget(
        Paragraph::new(Text::from(visible_lines)).style(Style::default()),
        chat_area,
    );

    // ── Input bar ───────────────────────────────────────────────────
    let (input_title, input_border_style) = if app.ai_input_focused {
        ("Input", Style::default())
    } else {
        ("Input (Esc to focus)", Style::default().fg(Color::DarkGray))
    };
    let input_block = Block::default()
        .borders(Borders::ALL)
        .title(input_title)
        .border_style(input_border_style);
    let input_inner = input_block.inner(input_area);
    frame.render_widget(input_block, input_area);

    let input_display = if app.ai_input.is_empty() {
        if app.ai_input_focused {
            "Type your message..."
        } else {
            "Press any key to type..."
        }
    } else {
        app.ai_input.as_str()
    };
    frame.render_widget(
        Paragraph::new(input_display).style(match app.ai_status {
            AiStatus::Waiting => Style::default().fg(Color::DarkGray),
            _ => Style::default(),
        }),
        input_inner,
    );

    // Show a visible cursor at the insertion point.
    if app.ai_status != AiStatus::Waiting {
        let cursor_col: u16 = app
            .ai_input
            .chars()
            .take(app.ai_input_cursor)
            .map(|c| c.width().unwrap_or(0) as u16)
            .sum();
        frame.set_cursor_position((input_inner.x + cursor_col, input_inner.y));
    }
}

/// Renders the footer bar for the AI tab.
fn render_ai_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let mut hint_spans = vec![
        footer_pill(" Type & Enter to send "),
        Span::raw(" "),
        footer_pill(" Esc clear input "),
        Span::raw(" "),
        footer_pill(" Quit [q] "),
    ];
    // Show tool execution status in the footer when the agent is acting
    if let Some(ref tool_status) = app.ai_tool_status {
        hint_spans.push(Span::raw("  "));
        hint_spans.push(Span::styled(
            tool_status.clone(),
            Style::default().fg(Color::Yellow),
        ));
    }
    let hints = Line::from(hint_spans);
    frame.render_widget(Paragraph::new(hints), area);
}

/// Renders the MindMap layout: tree (60%) + optional AI preview panel (40%).
fn render_mindmap_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if app.mindmap_ai_panel_visible && area.width >= 20 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);
        let tree_focused = app.mindmap_focus == MindMapFocus::Main;
        render_tree_panel(frame, app, cols[0], tree_focused);
        render_mindmap_ai_panel(frame, app, cols[1]);
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

fn render_explore_features(frame: &mut Frame<'_>, app: &App, area: Rect) {
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
}

fn render_explore_scenarios(frame: &mut Frame<'_>, app: &App, area: Rect) {
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

fn render_explore_steps(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let focused = app.explore_focus == ColumnFocus::Step;
    if app.explore_edit_mode {
        render_editor_panel(frame, app, area, false);
        return;
    }
    let block = explore_block("Steps", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let highlight_style = explore_select_style(focused);
    let mut lines: Vec<Line> = Vec::new();

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
    } else {
        let mut last_major: Option<Color> = None;
        if !background_steps.is_empty() {
            lines.push(Line::styled(
                " Background:",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ));
            for step in background_steps {
                let kw = format!("{:>6}", step.keyword);
                let kw_color = match step.keyword.as_str() {
                    "Given" => {
                        last_major = Some(KEYWORD_GIVEN);
                        KEYWORD_GIVEN
                    }
                    "When" => {
                        last_major = Some(KEYWORD_WHEN);
                        KEYWORD_WHEN
                    }
                    "Then" => {
                        last_major = Some(KEYWORD_THEN);
                        KEYWORD_THEN
                    }
                    "And" => last_major.unwrap_or(KEYWORD_AND),
                    "But" => last_major.unwrap_or(KEYWORD_BUT),
                    _ => Color::White,
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
            }
            lines.push(Line::raw(""));
        }

        if let Some(scenario) = scenario
            && !scenario.tags.is_empty()
        {
            lines.push(Line::styled(
                format!(" {}", scenario.tags.join(" ")),
                Style::default().fg(Color::DarkGray),
            ));
        }

        for (i, step) in scenario_steps.iter().enumerate() {
            let kw = format!("{:>6}", step.keyword);
            let kw_color = match step.keyword.as_str() {
                "Given" => {
                    last_major = Some(KEYWORD_GIVEN);
                    KEYWORD_GIVEN
                }
                "When" => {
                    last_major = Some(KEYWORD_WHEN);
                    KEYWORD_WHEN
                }
                "Then" => {
                    last_major = Some(KEYWORD_THEN);
                    KEYWORD_THEN
                }
                "And" => last_major.unwrap_or(KEYWORD_AND),
                "But" => last_major.unwrap_or(KEYWORD_BUT),
                _ => Color::White,
            };
            let kw_span = Span::styled(kw, Style::default().fg(kw_color));
            let body_span = Span::raw(format!(" {}", step.text));
            let mut line = Line::from(vec![kw_span, body_span]);
            if i == app.explore_selected_step {
                line = apply_line_background(line, highlight_style);
            }
            line = truncate_line_to_cols(line, inner.width);
            let trail = if i == app.explore_selected_step {
                highlight_style
            } else {
                Style::default()
            };
            line = pad_line_to_width(line, inner.width, trail);
            lines.push(line);
        }

        if let Some(scenario) = scenario
            && !scenario.examples.is_empty()
        {
            lines.push(Line::raw(""));
            for table in &scenario.examples {
                if !table.tags.is_empty() {
                    lines.push(Line::styled(
                        format!(" {}", table.tags.join(" ")),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                lines.push(Line::styled(
                    " Examples:",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ));
                for row in render_examples_table_lines(&table.headers, &table.rows) {
                    lines.push(Line::raw(format!("   {row}")));
                }
            }
        }
    }

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

    let tree = Tree::new(items)
        .expect("tree construction should succeed")
        .block(block)
        .highlight_style(highlight_style);

    frame.render_stateful_widget(tree, area, &mut app.tree_state);
}

/// Renders the AI preview panel in the MindMap tab.
fn render_mindmap_ai_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    use crate::app::{AiRole, AiStatus};

    if area.width < 10 || area.height < 3 {
        return;
    }

    let focused = app.mindmap_focus == MindMapFocus::AiPanel;
    let block = explore_block("AI Preview", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Status line
    let status_style = match app.ai_status {
        AiStatus::Waiting => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::SLOW_BLINK),
        AiStatus::Error => Style::default().fg(Color::Red),
        AiStatus::Idle => Style::default().fg(Color::DarkGray),
    };
    let status_text = match app.ai_status {
        AiStatus::Idle => "AI Ready",
        AiStatus::Waiting => "AI is thinking...",
        AiStatus::Error => "AI Error",
    };
    lines.push(Line::styled(status_text, status_style));
    lines.push(Line::raw(""));

    // Tool status
    if let Some(ref tool_status) = app.ai_tool_status {
        lines.push(Line::styled(
            tool_status.clone(),
            Style::default().fg(Color::Yellow),
        ));
        lines.push(Line::raw(""));
    }

    // Selected node context and filtered messages
    let node_text = crate::mindmap::selected_node_context(&app.tree_state, &app.mindmap_index)
        .map(|ctx| ctx.step_text)
        .unwrap_or_default();

    if !node_text.is_empty() {
        lines.push(Line::styled(
            format!("Selected: \"{node_text}\""),
            Style::default().fg(Color::Cyan),
        ));
        lines.push(Line::raw(""));

        let lower_node = node_text.to_lowercase();
        let matching: Vec<&crate::app::AiChatMessage> = app
            .ai_messages
            .iter()
            .filter(|msg| {
                !matches!(msg.role, AiRole::Tool)
                    && msg.content.to_lowercase().contains(&lower_node)
            })
            .collect();

        if matching.is_empty() {
            lines.push(Line::styled(
                "  No related AI messages",
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            lines.push(Line::styled(
                format!("Related messages ({}):", matching.len()),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ));
            for msg in matching.into_iter().take(8) {
                let prefix = match msg.role {
                    AiRole::User => "  \u{25b6} You",
                    AiRole::Assistant => "  \u{2726} AI",
                    AiRole::Tool => "  \u{2699} Tool",
                };
                let first = msg.content.lines().next().unwrap_or("");
                let truncated =
                    truncate_string_to_cols(first, inner.width.saturating_sub(4) as u16);
                lines.push(Line::raw(format!("{prefix}: {truncated}")));
            }
        }
        lines.push(Line::raw(""));
    }

    // Command hints
    let hint_style = Style::default().fg(Color::DarkGray);
    if focused {
        lines.push(Line::styled("Esc / Left  return to tree", hint_style));
    } else {
        lines.push(Line::styled("Enter  focus AI preview", hint_style));
    }
    lines.push(Line::styled("Ctrl+\\  toggle panel", hint_style));

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
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
        let _ = highlight_line_with_state(&line, &mut step_state, &KeywordSet::default());
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
            step_line_display(&display_line, step_state.in_doc_string);
        let display_len = display_line.chars().count();
        let mut styled =
            highlight_line_with_state(&display_line, &mut step_state, &KeywordSet::default());

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
    let max_kw_ch = STEP_KEYWORDS_CYCLE
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(5);
    let inner_w_ch = max_kw_ch.max(TITLE.chars().count()).saturating_add(2);
    let list_w = (inner_w_ch as u16).saturating_add(2);
    let n_items = STEP_KEYWORDS_CYCLE.len();
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
    for (i, kw) in STEP_KEYWORDS_CYCLE.iter().enumerate().take(max_rows) {
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
        (MainTab::MindMap, _) => {
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
                if app.mindmap_focus == MindMapFocus::Main {
                    hints.push(Span::raw(" "));
                    hints.push(footer_pill(" Focus Panel [Enter] "));
                }
                hints.push(Span::raw(" "));
                hints.push(footer_pill(" Toggle Panel [Ctrl+\\] "));
            } else {
                hints.push(Span::raw(" "));
                hints.push(footer_pill(" Show Panel [Ctrl+\\] "));
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
        (MainTab::Help, _) => Line::from(vec![
            footer_pill(" Save [s] "),
            Span::raw(" "),
            footer_pill(" Quit [q] "),
        ]),
        (MainTab::Ai, _) => Line::from(vec![footer_pill(" Quit [q] ")]),
    }
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
