use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Text;
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::highlight::{KeywordSet, highlight_line};

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let title = match &app.file_path {
        Some(path) => format!(" teshi - {} ", path.display()),
        None => " teshi - [No file] ".to_string(),
    };
    frame.render_widget(
        Paragraph::new(title).style(Style::default().add_modifier(Modifier::BOLD)),
        chunks[0],
    );

    let editor_block = Block::default().borders(Borders::ALL).title("BDD Editor");
    let editor_area = editor_block.inner(chunks[1]);
    frame.render_widget(editor_block, chunks[1]);

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
            lines.push(ratatui::text::Line::raw(String::new()));
            continue;
        }
        let line = app.buffer.line(row);
        let (styled, next_doc) = highlight_line(&line, in_doc, &KeywordSet::default());
        in_doc = next_doc;
        lines.push(styled);
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), editor_area);

    let line_num = app.cursor_row + 1;
    let col_num = app.cursor_col + 1;
    let dirty = if app.dirty { "modified" } else { "saved" };
    let mode = if app.step_input_active {
        "STEP_INPUT"
    } else {
        "NAV"
    };
    let status = format!(
        " {} | {} | Ln {}, Col {} | {}",
        app.status, mode, line_num, col_num, dirty
    );
    frame.render_widget(Paragraph::new(status), chunks[2]);

    let cursor_line = app.buffer.line(app.cursor_row);
    let mut visual_col = 0usize;
    for ch in cursor_line.chars().take(app.cursor_col) {
        visual_col += UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]));
    }
    let x = editor_area.x + visual_col as u16;
    let y = editor_area.y + (app.cursor_row.saturating_sub(app.scroll_row)) as u16;
    frame.set_cursor_position((x, y));
}
