use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs};
use tui_tree_widget::Tree;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, BddFocusSlot, ColumnFocus, MainTab, STEP_KEYWORDS_CYCLE, ViewStage};
use crate::bdd_nav::{is_feature_narrative_row, keyword_char_range, nav_body_char_range_in_buffer};
use crate::highlight::{KeywordSet, highlight_line};

const NAV_CELL_BG: Color = Color::LightBlue;
const NAV_CELL_FG: Color = Color::Black;
/// Pale background for the focused keyword or step body in navigation mode.
const NODE_FOCUS_BG: Color = Color::Rgb(140, 190, 255);
/// Stage-2 preview: one solid style for the tree-selected line (avoids span-patch gaps that read as bright blocks).
const PREVIEW_CURSOR_BG: Color = Color::DarkGray;
const PREVIEW_CURSOR_FG: Color = Color::White;
const STATUS_PENDING: Color = Color::DarkGray;
const KEYWORD_GIVEN: Color = Color::Blue;
const KEYWORD_WHEN: Color = Color::Yellow;
const KEYWORD_THEN: Color = Color::Green;
const KEYWORD_AND: Color = Color::Gray;
const KEYWORD_BUT: Color = Color::Gray;

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
        Line::from(" MindMap [1] "),
        Line::from(" Explore [2] "),
        Line::from(" Help [3] "),
    ])
    .select(match app.active_tab {
        MainTab::MindMap => 0,
        MainTab::Explore => 1,
        MainTab::Help => 2,
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

    if app.active_tab == MainTab::Explore {
        render_explore_footer(frame, app, chunks[3]);
    } else {
        let key_hints = footer_hints(app);
        frame.render_widget(Paragraph::new(key_hints), chunks[3]);
    }
}

fn render_main_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    match app.active_tab {
        MainTab::MindMap => render_mindmap_panel(frame, app, area),
        MainTab::Explore => render_explore_panel(frame, app, area),
        MainTab::Help => {
            let block = Block::default().borders(Borders::ALL).title("Help");
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let help = vec![
                Line::raw("Tabs: MindMap [1], Explore [2], Help [3]"),
                Line::raw(""),
                Line::raw("── MindMap (Stage 1: Tree only) ──"),
                Line::raw(
                    "↑↓ navigate tree   Space toggle   ←→ collapse/expand   Enter open preview",
                ),
                Line::raw("Home/End first/last node"),
                Line::raw(""),
                Line::raw("── MindMap (Stage 2: Tree + Preview) ──"),
                Line::raw("↑↓ navigate tree   Space toggle   ← collapse"),
                Line::raw("→ enter editor   Enter/Esc close preview"),
                Line::raw("[ / ] cycle preview location"),
                Line::raw(""),
                Line::raw("── MindMap (Stage 3: Editor) ──"),
                Line::raw("↑↓ BDD nav   ←→ keyword/body focus   Space edit"),
                Line::raw("← on keyword: back to tree   Esc: clear input / back"),
                Line::raw(""),
                Line::raw("── Explore (Three Columns) ──"),
                Line::raw("Tab/Shift+Tab/←→ switch column   ↑↓ navigate   e edit   r run   a AI"),
                Line::raw("Esc exit edit"),
                Line::raw(""),
                Line::raw("Global: s save, q quit (dirty needs confirmation)"),
            ];
            frame.render_widget(Paragraph::new(Text::from(help)), inner);
        }
    }
}

/// Renders the three-stage MindMap layout.
fn render_mindmap_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    match app.view_stage {
        ViewStage::TreeOnly => {
            render_tree_panel(frame, app, area);
        }
        ViewStage::TreeAndEditor => {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
                .split(area);
            render_tree_panel(frame, app, cols[0]);
            // Reset the whole preview column so the location strip and editor cannot leave ghosts
            // from prior frames or widgets (ratatui diffs against the previous buffer only).
            frame.render_widget(Clear, cols[1]);
            if app.current_location_info().is_some() {
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(1)])
                    .split(cols[1]);
                render_location_panel(frame, app, rows[0]);
                render_editor_panel(frame, app, rows[1], true);
            } else {
                render_editor_panel(frame, app, cols[1], true);
            }
        }
        ViewStage::EditorAndPanel => {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
                .split(area);
            render_editor_panel(frame, app, cols[0], false);
            render_reserved_panel(frame, cols[1]);
        }
    }
}

/// Renders the Explore tab: three-column feature/scenario/step browser.
fn render_explore_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
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
}

fn explore_select_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .bg(NAV_CELL_BG)
            .fg(NAV_CELL_FG)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    }
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
    let block = explore_block("Scenarios", focused);
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
            let status_dot = Span::styled("●", Style::default().fg(STATUS_PENDING));
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

    let steps = app
        .project
        .features
        .get(app.explore_selected_feature)
        .and_then(|f| f.scenarios.get(app.explore_selected_scenario))
        .map(|s| &s.steps);

    if steps.is_none_or(|s| s.is_empty()) {
        lines.push(Line::styled(
            " (no steps)",
            Style::default().fg(Color::DarkGray),
        ));
    } else if let Some(steps) = steps {
        for (i, step) in steps.iter().enumerate() {
            let kw = format!("{:>6}", step.keyword);
            let kw_color = match step.keyword.as_str() {
                "Given" => KEYWORD_GIVEN,
                "When" => KEYWORD_WHEN,
                "Then" => KEYWORD_THEN,
                "And" => KEYWORD_AND,
                "But" => KEYWORD_BUT,
                _ => Color::White,
            };
            let kw_span = Span::styled(kw, Style::default().fg(kw_color));
            let body_span = Span::raw(format!(" {}", step.text));
            let status_span = Span::styled(" pending", Style::default().fg(STATUS_PENDING));
            let mut line = Line::from(vec![kw_span, body_span, status_span]);
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
fn render_tree_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let items = &app.mindmap_index.items;

    let highlight_style = Style::default()
        .bg(NAV_CELL_BG)
        .fg(NAV_CELL_FG)
        .add_modifier(Modifier::BOLD);

    let block = Block::default().borders(Borders::ALL).title("MindMap");

    let tree = Tree::new(items)
        .expect("tree construction should succeed")
        .block(block)
        .highlight_style(highlight_style);

    frame.render_stateful_widget(tree, area, &mut app.tree_state);
}

fn render_location_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(info) = app.current_location_info() else {
        return;
    };
    let mut text = format!(
        " Location {}/{}  {}",
        info.index + 1,
        info.total,
        info.label
    );
    let width = area.width as usize;
    let used = UnicodeWidthStr::width(text.as_str());
    if used < width {
        text.push_str(&" ".repeat(width - used));
    }
    let loc_style = Style::default().bg(Color::DarkGray).fg(Color::White);
    frame
        .buffer_mut()
        .set_string(area.x, area.y, text, loc_style);
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
    let mut scroll_row = if preview {
        app.preview_scroll_row
    } else {
        app.scroll_row
    };
    if !preview {
        if cursor_row < scroll_row {
            scroll_row = cursor_row;
        } else if cursor_row >= scroll_row.saturating_add(visible_lines) {
            scroll_row = cursor_row.saturating_sub(visible_lines.saturating_sub(1));
        }
    } else {
        // In preview mode, center the cursor row
        scroll_row = cursor_row.saturating_sub(visible_lines / 2);
    }

    let mut lines = Vec::with_capacity(visible_lines);
    let preview_row_style = Style::default().bg(PREVIEW_CURSOR_BG).fg(PREVIEW_CURSOR_FG);
    let mut in_doc = false;
    for row in 0..scroll_row {
        if row >= buffer.line_count() {
            break;
        }
        let line = buffer.line(row);
        let (_, next_doc) = highlight_line(&line, in_doc, &KeywordSet::default());
        in_doc = next_doc;
    }
    for row in scroll_row..scroll_row.saturating_add(visible_lines) {
        if row >= buffer.line_count() {
            let empty = pad_line_to_width(
                Line::raw(String::new()),
                editor_area.width,
                Style::default(),
            );
            lines.push(empty);
            continue;
        }
        let line = buffer.line(row);
        let (mut styled, next_doc) = highlight_line(&line, in_doc, &KeywordSet::default());
        in_doc = next_doc;

        if row == cursor_row && !preview {
            let nav_cell_style = Style::default().bg(NAV_CELL_BG).fg(NAV_CELL_FG);
            let line_len = line.chars().count();
            if app.is_editor_nav_mode() {
                let focus_patch = Style::default().bg(NODE_FOCUS_BG);
                let hl_range = match app.focus_slot {
                    BddFocusSlot::Keyword => keyword_char_range(&line).or_else(|| {
                        if is_feature_narrative_row(buffer, row) {
                            nav_body_char_range_in_buffer(buffer, row, &line)
                        } else {
                            None
                        }
                    }),
                    BddFocusSlot::Body => nav_body_char_range_in_buffer(buffer, row, &line),
                };
                if let Some(r) = hl_range
                    && r.start < r.end
                {
                    styled = apply_patch_to_char_range(styled, r, focus_patch);
                }
            } else if app.step_input_active || app.step_keyword_picker.is_some() {
                if line_len == 0 {
                    styled = Line::from(vec![Span::styled(" ", nav_cell_style)]);
                } else if app.cursor_col < line_len {
                    styled = apply_patch_to_char_range(
                        styled,
                        app.cursor_col..app.cursor_col.saturating_add(1),
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
            styled = if line.is_empty() {
                Line::from(vec![Span::styled(" ", preview_row_style)])
            } else {
                Line::from(Span::styled(line.to_string(), preview_row_style))
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
            let buffer_row = scroll_row.saturating_add(i);
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
        app.preview_scroll_row = scroll_row;
    } else {
        app.scroll_row = scroll_row;
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

    let selected_style = Style::default().bg(NAV_CELL_BG).fg(NAV_CELL_FG);
    let normal = Style::default();

    let mut lines: Vec<Line> = Vec::with_capacity(max_rows.min(n_items));
    for (i, kw) in STEP_KEYWORDS_CYCLE.iter().enumerate().take(max_rows) {
        let style = if i == picker.selected {
            selected_style
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
fn render_reserved_panel(frame: &mut Frame<'_>, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Reserved")
        .style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let content = vec![
        Line::raw(""),
        Line::styled(
            "  Coming Soon",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::raw("  Planned:"),
        Line::raw("  · Step impl code"),
        Line::raw("  · BDD executor"),
        Line::raw("  · Test results"),
    ];
    frame.render_widget(Paragraph::new(Text::from(content)), inner);
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
    let right = format!("0/{total} 通过");

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
    Span::styled(
        label,
        Style::default().bg(Color::LightBlue).fg(Color::Black),
    )
}

fn footer_hints(app: &App) -> Line<'static> {
    match (app.active_tab, app.view_stage) {
        (MainTab::MindMap, ViewStage::TreeOnly) => Line::from(vec![
            footer_pill(" Navigate [↑↓] "),
            Span::raw(" "),
            footer_pill(" Toggle [Space] "),
            Span::raw(" "),
            footer_pill(" Expand [→] "),
            Span::raw(" "),
            footer_pill(" Collapse [←] "),
            Span::raw(" "),
            footer_pill(" Open [Enter] "),
            Span::raw(" "),
            footer_pill(" Quit [q] "),
        ]),
        (MainTab::MindMap, ViewStage::TreeAndEditor) => Line::from(vec![
            footer_pill(" Navigate [↑↓] "),
            Span::raw(" "),
            footer_pill(" Toggle [Space] "),
            Span::raw(" "),
            footer_pill(" Edit [→] "),
            Span::raw(" "),
            footer_pill(" Location [[/]] "),
            Span::raw(" "),
            footer_pill(" Close [Enter] "),
            Span::raw(" "),
            footer_pill(" Back [Esc] "),
            Span::raw(" "),
            footer_pill(" Save [s] "),
            Span::raw(" "),
            footer_pill(" Quit [q] "),
        ]),
        (MainTab::MindMap, ViewStage::EditorAndPanel) => Line::from(vec![
            footer_pill(" BDD [←→↑↓] "),
            Span::raw(" "),
            footer_pill(" Step [Space] "),
            Span::raw(" "),
            footer_pill(" Back [← kw] "),
            Span::raw(" "),
            footer_pill(" Save [s] "),
            Span::raw(" "),
            footer_pill(" Quit [q] "),
            Span::raw(" "),
            footer_pill(" Clear [Esc] "),
        ]),
        (MainTab::Explore, _) => Line::from(vec![
            footer_pill(" Focus [Tab/←→] "),
            Span::raw(" "),
            footer_pill(" Navigate [↑↓] "),
            Span::raw(" "),
            footer_pill(" Edit [e] "),
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
    }
}

#[cfg(test)]
mod truncate_tests {
    use super::{Line, Span, truncate_line_to_cols};

    #[test]
    fn truncate_line_to_cols_limits_display_width() {
        let line = Line::from(Span::raw("a".repeat(100)));
        assert!(line.width() > 68);
        let out = truncate_line_to_cols(line, 68);
        assert!(out.width() <= 68);
    }
}
