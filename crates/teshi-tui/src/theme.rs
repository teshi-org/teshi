//! Shared chrys-derived palette and component styles for the terminal UI.
//!
//! Values follow the default `chrys` theme (a Dracula-inspired purple palette).
//! The terminal background is intentionally never painted so the chat blends
//! into the user's own terminal theme; only block surfaces use `BLOCK_BG`.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};

use crate::app::RunStatus;
use teshi_core::git::DiffStatus;

/// Main brand color: headings, focused borders, bullets, links.
pub(crate) const PRIMARY: Color = Color::Rgb(0xAF, 0x87, 0xFF);
/// Subdued primary for quote bars and other low-emphasis brand accents.
pub(crate) const PRIMARY_MUTED: Color = Color::Rgb(0x87, 0x5F, 0xAF);
/// Secondary brand color used for horizontal rules.
pub(crate) const SECONDARY: Color = Color::Rgb(0x5F, 0x5F, 0xAF);
/// Accent color for user messages, the input box, and the thinking spinner.
pub(crate) const ACCENT: Color = Color::Rgb(0xFF, 0x87, 0xD7);
/// Success state: assistant rail, completed tools, added diff lines.
pub(crate) const SUCCESS: Color = Color::Rgb(0x5F, 0xFF, 0x87);
/// Warning state: inline code, pending work, modified diff lines.
pub(crate) const WARNING: Color = Color::Rgb(0xFF, 0xAF, 0x5F);
/// Error state: failed tools, deleted diff lines, error messages.
pub(crate) const ERROR: Color = Color::Rgb(0xFF, 0x5F, 0x5F);
/// Default emphasized foreground (headings H4/H5).
pub(crate) const FG: Color = Color::Rgb(0xEE, 0xEE, 0xEE);
/// Muted foreground for hints, labels, and secondary metadata.
pub(crate) const MUTED: Color = Color::Rgb(0x94, 0x94, 0x94);
/// Quiet border color for unfocused panels and table rules.
pub(crate) const BORDER: Color = Color::Rgb(0x5F, 0x5F, 0x5F);
/// Surface color for code fences and blockquotes.
pub(crate) const BLOCK_BG: Color = Color::Rgb(0x30, 0x30, 0x30);
/// Scrollbar thumb color.
pub(crate) const SCROLL_THUMB: Color = Color::Rgb(0x58, 0x58, 0x58);
/// Scrollbar track color.
pub(crate) const SCROLL_TRACK: Color = Color::Rgb(0x3A, 0x3A, 0x3A);
/// Background of the selected row in lists and option pickers.
pub(crate) const CURSOR_BG: Color = Color::Rgb(0x87, 0x87, 0xD7);
/// Foreground of the selected row in lists and option pickers.
pub(crate) const CURSOR_FG: Color = Color::Rgb(0x30, 0x30, 0x30);
/// Foreground used on solid-colored chips such as the approval badge.
pub(crate) const CHIP_FG: Color = Color::Rgb(0x1C, 0x1C, 0x1C);

/// Full-width footer strip.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) const FOOTER_BG: Color = Color::Rgb(0x26, 0x26, 0x26);
/// Footer hint descriptions.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) const FOOTER_DESC: Color = Color::Rgb(0xD0, 0xD0, 0xD0);
/// Active tab surface.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) const TAB_ACTIVE_BG: Color = Color::Rgb(0x41, 0x37, 0x55);
/// Inactive tabs and continuation keywords.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) const TAB_INACTIVE: Color = Color::Rgb(0xB2, 0xB2, 0xB2);
/// Selection retained outside the focused widget.
pub(crate) const SELECTION_INACTIVE_BG: Color = Color::Rgb(0x44, 0x44, 0x44);
/// Selected editor text.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) const EDITOR_SELECTION_BG: Color = Color::Rgb(0x5F, 0x5F, 0x87);
/// Current preview line surface.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) const CURRENT_LINE_BG: Color = Color::Rgb(0x30, 0x30, 0x30);
/// Overlay and toast surface.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) const OVERLAY_BG: Color = Color::Rgb(0x3A, 0x3A, 0x3A);
/// Added-line tint.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) const DIFF_ADDED_BG: Color = Color::Rgb(0x26, 0x3E, 0x2C);
/// Modified-line tint.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) const DIFF_MODIFIED_BG: Color = Color::Rgb(0x3E, 0x32, 0x26);
/// Deleted-line tint.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) const DIFF_DELETED_BG: Color = Color::Rgb(0x3E, 0x26, 0x26);
/// Gherkin string literals.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) const SYNTAX_STRING: Color = Color::Rgb(0xD7, 0xAF, 0x87);

/// Rounded panel chrome; callers retain their existing interior geometry.
pub(crate) fn panel_block<'a>(title: impl Into<Line<'a>>, focused: bool) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if focused { PRIMARY } else { BORDER }))
        .title(title)
        .title_style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD))
}

/// Popup chrome; the caller must clear the destination before rendering.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) fn popup_block<'a>(title: impl Into<Line<'a>>) -> Block<'a> {
    panel_block(title, true)
}

/// Selection fill to apply across the entire row.
pub(crate) fn selected_row(focused: bool) -> Style {
    if focused {
        Style::default()
            .bg(CURSOR_BG)
            .fg(CURSOR_FG)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(SELECTION_INACTIVE_BG).fg(FG)
    }
}

/// Solid badge retaining the caller's exact label.
pub(crate) fn chip<'a>(label: impl Into<std::borrow::Cow<'a, str>>, bg: Color) -> Span<'a> {
    Span::styled(
        label,
        Style::default()
            .bg(bg)
            .fg(CHIP_FG)
            .add_modifier(Modifier::BOLD),
    )
}

/// Key and description spans retaining the supplied punctuation and spacing.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) fn key_hint<'a>(key: &'a str, desc: &'a str) -> Vec<Span<'a>> {
    vec![
        Span::styled(
            key,
            Style::default()
                .fg(PRIMARY)
                .bg(FOOTER_BG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc, Style::default().fg(FOOTER_DESC).bg(FOOTER_BG)),
    ]
}

/// Footer line; paragraph styling should extend its background to the full strip.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) fn footer_line<'a>(hints: impl IntoIterator<Item = (&'a str, &'a str)>) -> Line<'a> {
    Line::from(
        hints
            .into_iter()
            .flat_map(|(key, desc)| key_hint(key, desc))
            .collect::<Vec<_>>(),
    )
    .style(Style::default().fg(FOOTER_DESC).bg(FOOTER_BG))
}

/// Low-emphasis metadata style.
pub(crate) fn muted() -> Style {
    Style::default().fg(MUTED)
}

/// Semantic run state foreground.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) fn status_style(status: RunStatus) -> Style {
    Style::default().fg(match status {
        RunStatus::Idle => MUTED,
        RunStatus::Running => WARNING,
        RunStatus::Passed => SUCCESS,
        RunStatus::Failed => ERROR,
        RunStatus::Skipped => BORDER,
    })
}

/// Semantic diff foreground without imposing a background on unchanged rows.
// Consumed by subsequent surface migrations in adopt-chrys-tui-style.
#[allow(dead_code)]
pub(crate) fn diff_style(kind: DiffStatus) -> Style {
    Style::default().fg(match kind {
        DiffStatus::Added => SUCCESS,
        DiffStatus::Modified => WARNING,
        DiffStatus::Deleted => ERROR,
        DiffStatus::Unchanged => MUTED,
    })
}

/// Vertical scrollbar rows; empty when content fits, with out-of-range starts clamped.
pub(crate) fn scrollbar_lines(total: usize, height: usize, start: usize) -> Vec<Line<'static>> {
    if height == 0 || total <= height {
        return Vec::new();
    }
    let thumb_len = height
        .saturating_mul(height)
        .div_ceil(total)
        .clamp(1, height);
    let track = height - thumb_len;
    let scrollable = total - height;
    let thumb_pos = start.min(scrollable).saturating_mul(track) / scrollable;
    (0..height)
        .map(|i| {
            if (thumb_pos..thumb_pos + thumb_len).contains(&i) {
                Line::styled("┃", Style::default().fg(SCROLL_THUMB))
            } else {
                Line::styled("│", Style::default().fg(SCROLL_TRACK))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn test_panel_chrome_and_selection() {
        for focused in [false, true] {
            let mut terminal = Terminal::new(TestBackend::new(12, 4)).unwrap();
            terminal
                .draw(|frame| frame.render_widget(panel_block("Title", focused), frame.area()))
                .unwrap();
            let corner = &terminal.backend().buffer()[(0, 0)];
            assert_eq!(corner.symbol(), "╭");
            assert_eq!(corner.fg, if focused { PRIMARY } else { BORDER });
            let style = selected_row(focused);
            assert_eq!(
                style.bg,
                Some(if focused {
                    CURSOR_BG
                } else {
                    SELECTION_INACTIVE_BG
                })
            );
            assert_eq!(style.fg, Some(if focused { CURSOR_FG } else { FG }));
        }
    }

    #[test]
    fn test_scrollbar_bounds() {
        assert!(scrollbar_lines(10, 0, 0).is_empty());
        assert!(scrollbar_lines(4, 4, 0).is_empty());
        assert_eq!(
            scrollbar_lines(10, 4, usize::MAX),
            scrollbar_lines(10, 4, 6)
        );
        assert_eq!(scrollbar_lines(10, 4, 0)[0].spans[0].content, "┃");
        assert_eq!(scrollbar_lines(10, 4, 6)[3].spans[0].content, "┃");
    }

    #[test]
    fn test_page_color_budget() {
        const CAPS: &[(&str, &str, usize)] = &[
            ("ui.rs", include_str!("ui.rs"), 182),
            ("highlight.rs", include_str!("highlight.rs"), 14),
            ("mindmap.rs", include_str!("mindmap.rs"), 19),
            ("authoring_tab.rs", include_str!("authoring_tab.rs"), 10),
            ("test_points_tab.rs", include_str!("test_points_tab.rs"), 10),
            ("ui/chat.rs", include_str!("ui/chat.rs"), 1),
            ("markdown.rs", include_str!("markdown.rs"), 0),
        ];
        for &(path, source, cap) in CAPS {
            let count = source.matches("Color::").count();
            assert!(count <= cap, "{path}: {count} raw colors exceeds cap {cap}");
        }
    }
}
