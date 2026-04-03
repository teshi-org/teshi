use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, BddFocusSlot, MainTab, STEP_KEYWORDS_CYCLE};
use crate::bdd_nav::{body_char_range, keyword_char_range};
use crate::highlight::{KeywordSet, highlight_line};

const NAV_CELL_BG: Color = Color::LightBlue;
const NAV_CELL_FG: Color = Color::Black;
/// Pale background for the focused keyword or step body in navigation mode.
const NODE_FOCUS_BG: Color = Color::Rgb(140, 190, 255);

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
        Line::from(" Editor [1] "),
        Line::from(" Feature [2] "),
        Line::from(" Help [3] "),
    ])
    .select(match app.active_tab {
        MainTab::Editor => 0,
        MainTab::Feature => 1,
        MainTab::Help => 2,
    })
    .style(Style::default().fg(Color::DarkGray))
    // Avoid Underlined: many IDE terminals emulate it with leading/trailing `_`, which looks like stray punctuation.
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

    let key_hints = footer_hints(app.active_tab);
    frame.render_widget(Paragraph::new(key_hints), chunks[3]);
}

fn render_main_panel(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    match app.active_tab {
        MainTab::Editor => render_editor_panel(frame, app, area),
        MainTab::Feature => {
            let block = Block::default()
                .borders(Borders::ALL)
                .title("Feature Outline");
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let lines = app
                .feature_outline_lines()
                .into_iter()
                .map(Line::raw)
                .collect::<Vec<_>>();
            frame.render_widget(Paragraph::new(Text::from(lines)), inner);
            inner
        }
        MainTab::Help => {
            let block = Block::default().borders(Borders::ALL).title("Help");
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let help = vec![
                Line::raw("Tabs: Editor [1], Feature [2], Help [3]"),
                Line::raw("Editor: ↑↓ jump between BDD nodes; ←/→ switch keyword vs step body"),
                Line::raw(
                    "Editor: Space on keyword opens step list; Space on body edits step text",
                ),
                Line::raw(
                    "Editor: ↑↓ in list, Enter confirm, Esc cancel; Enter commits step input",
                ),
                Line::raw("Global: s save, q quit (dirty needs confirmation)"),
            ];
            frame.render_widget(Paragraph::new(Text::from(help)), inner);
            inner
        }
    }
}

fn render_editor_panel(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let editor_block = Block::default().borders(Borders::ALL).title("BDD Editor");
    let editor_area = editor_block.inner(area);
    frame.render_widget(editor_block, area);

    let visible_lines = editor_area.height as usize;
    if app.cursor_row < app.scroll_row {
        app.scroll_row = app.cursor_row;
    } else if app.cursor_row >= app.scroll_row.saturating_add(visible_lines) {
        app.scroll_row = app
            .cursor_row
            .saturating_sub(visible_lines.saturating_sub(1));
    }

    let mut lines = Vec::with_capacity(visible_lines);
    let mut in_doc = false;
    for row in 0..app.scroll_row {
        let line = app.buffer.line(row);
        let (_, next_doc) = highlight_line(&line, in_doc, &KeywordSet::default());
        in_doc = next_doc;
    }
    for row in app.scroll_row..app.scroll_row.saturating_add(visible_lines) {
        if row >= app.buffer.line_count() {
            lines.push(Line::raw(String::new()));
            continue;
        }
        let line = app.buffer.line(row);
        let (mut styled, next_doc) = highlight_line(&line, in_doc, &KeywordSet::default());
        in_doc = next_doc;
        if row == app.cursor_row {
            let nav_cell_style = Style::default().bg(NAV_CELL_BG).fg(NAV_CELL_FG);
            let line_len = line.chars().count();
            if app.active_tab == MainTab::Editor
                && !app.step_input_active
                && app.step_keyword_picker.is_none()
            {
                let focus_patch = Style::default().bg(NODE_FOCUS_BG);
                let hl_range = match app.focus_slot {
                    BddFocusSlot::Keyword => keyword_char_range(&line),
                    BddFocusSlot::Body => body_char_range(&line),
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
        }
        lines.push(styled);
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), editor_area);
    render_step_keyword_picker(frame, app, editor_area);
    editor_area
}

/// Draws the step-keyword overlay when [`App::step_keyword_picker`] is active.
///
/// Uses [`Clear`] plus a [`Paragraph`] so underlying editor cells are not merged with list text
/// (narrow [`List`] + stale buffer cells produced strings like "Givend" / "Whenhen").
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

/// One footer “button” like gitui: light blue background and black foreground.
fn footer_pill(label: &'static str) -> Span<'static> {
    Span::styled(
        label,
        Style::default().bg(Color::LightBlue).fg(Color::Black),
    )
}

fn footer_hints(active_tab: MainTab) -> Line<'static> {
    match active_tab {
        MainTab::Editor => Line::from(vec![
            footer_pill(" BDD [←→↑↓] "),
            Span::raw(" "),
            footer_pill(" Step [Space] "),
            Span::raw(" "),
            footer_pill(" Save [s] "),
            Span::raw(" "),
            footer_pill(" Quit [q] "),
            Span::raw(" "),
            footer_pill(" Clear [Esc] "),
        ]),
        MainTab::Feature | MainTab::Help => Line::from(vec![
            footer_pill(" Save [s] "),
            Span::raw(" "),
            footer_pill(" Quit [q] "),
        ]),
    }
}
