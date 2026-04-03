use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, MainTab};
use crate::highlight::{KeywordSet, highlight_line};

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

    let editor_area = render_main_panel(frame, app, chunks[2]);

    let key_hints = footer_hints(app.active_tab);
    frame.render_widget(Paragraph::new(key_hints), chunks[3]);

    if app.active_tab == MainTab::Editor {
        let cursor_line = app.buffer.line(app.cursor_row);
        let mut visual_col = 0usize;
        for ch in cursor_line.chars().take(app.cursor_col) {
            visual_col += UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]));
        }
        let x = editor_area.x + visual_col as u16;
        let y = editor_area.y + (app.cursor_row.saturating_sub(app.scroll_row)) as u16;
        frame.set_cursor_position((x, y));
    }
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
                Line::raw("Editor: Arrow keys move cursor"),
                Line::raw("Editor: Space activates step text input"),
                Line::raw("Editor: Enter commits step input, Esc clears input state"),
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
        let (styled, next_doc) = highlight_line(&line, in_doc, &KeywordSet::default());
        in_doc = next_doc;
        lines.push(styled);
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), editor_area);
    editor_area
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
            footer_pill(" Move [←→↑↓] "),
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
