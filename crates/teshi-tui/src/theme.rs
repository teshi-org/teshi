//! Shared chrys-derived palette and component styles for the terminal UI.
//!
//! Values follow the default `chrys` theme (a Dracula-inspired purple palette).
//! The terminal background stays visible. Fills are limited to component
//! surfaces such as selection, chips, footers, overlays, code and diff tints.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};

use crate::app::RunStatus;
use teshi_core::git::DiffStatus;

/// Maps existing named MindMap highlight rules onto the shared palette.
pub(crate) fn highlight_color(color: teshi_core::mindmap::HighlightColor) -> Color {
    use teshi_core::mindmap::HighlightColor;
    match color {
        HighlightColor::Red => ERROR,
        HighlightColor::Green => SUCCESS,
        HighlightColor::Yellow => WARNING,
        HighlightColor::Blue => SECONDARY,
        HighlightColor::Magenta => ACCENT,
        HighlightColor::Cyan => PRIMARY,
        HighlightColor::White => FG,
        HighlightColor::Black => CHIP_FG,
    }
}

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
/// Subtle user-message surface, restricted to the chat transcript.
pub(crate) const CHAT_USER_BG: Color = Color::Rgb(0x2B, 0x24, 0x2E);
/// Quiet purple surface for chat selectors and the selected conversation.
pub(crate) const CHAT_CHIP_BG: Color = Color::Rgb(0x33, 0x2C, 0x41);
/// Scrollbar thumb color.
pub(crate) const SCROLL_THUMB: Color = Color::Rgb(0x58, 0x58, 0x58);
/// Fill behind a scrollbar thumb, matching iCode's `scrollbar-background` so a
/// bar reads as a dark strip beside the content it scrolls.
pub(crate) const SCROLL_TRACK_BG: Color = Color::Rgb(0x1C, 0x1C, 0x1C);
/// Scrollbar thumb under the pointer. Lifted by the same headroom as
/// [`hover_fill`], because the track fill has no glyph to brighten.
pub(crate) const SCROLL_THUMB_HOVER: Color = Color::Rgb(0x76, 0x76, 0x76);
/// Scrollbar thumb while it is being dragged.
pub(crate) const SCROLL_THUMB_ACTIVE: Color = Color::Rgb(0x80, 0x80, 0x80);
/// Background of the selected row in lists and option pickers.
pub(crate) const CURSOR_BG: Color = Color::Rgb(0x87, 0x87, 0xD7);
/// Foreground of the selected row in lists and option pickers.
pub(crate) const CURSOR_FG: Color = Color::Rgb(0x30, 0x30, 0x30);
/// Foreground used on solid-colored chips such as the approval badge.
pub(crate) const CHIP_FG: Color = Color::Rgb(0x1C, 0x1C, 0x1C);

/// Full-width footer strip.
pub(crate) const FOOTER_BG: Color = Color::Rgb(0x26, 0x26, 0x26);
/// Footer hint descriptions.
pub(crate) const FOOTER_DESC: Color = Color::Rgb(0xD0, 0xD0, 0xD0);
/// Active tab surface.
pub(crate) const TAB_ACTIVE_BG: Color = Color::Rgb(0x41, 0x37, 0x55);
/// Inactive tabs and continuation keywords.
pub(crate) const TAB_INACTIVE: Color = Color::Rgb(0xB2, 0xB2, 0xB2);
/// Selection retained outside the focused widget.
pub(crate) const SELECTION_INACTIVE_BG: Color = Color::Rgb(0x44, 0x44, 0x44);
/// Surface tinted under the pointer for clickable regions.
pub(crate) const HOVER_BG: Color = Color::Rgb(0x3A, 0x3A, 0x3A);
/// Selected editor text.
pub(crate) const EDITOR_SELECTION_BG: Color = Color::Rgb(0x5F, 0x5F, 0x87);
/// Current preview line surface.
pub(crate) const CURRENT_LINE_BG: Color = Color::Rgb(0x30, 0x30, 0x30);
/// Overlay and toast surface.
pub(crate) const OVERLAY_BG: Color = Color::Rgb(0x3A, 0x3A, 0x3A);
/// Gherkin string literals.
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
pub(crate) fn popup_block<'a>(title: impl Into<Line<'a>>) -> Block<'a> {
    panel_block(title, true).style(Style::default().bg(OVERLAY_BG))
}

/// Extends a selected option's fill across its row, including styled metadata.
pub(crate) fn popup_rows<'a>(lines: Vec<Line<'a>>, width: u16) -> Vec<Line<'a>> {
    lines
        .into_iter()
        .map(|mut line| {
            let background = std::iter::once(line.style.bg)
                .chain(line.spans.iter().map(|span| span.style.bg))
                .flatten()
                .find(|bg| *bg == CURSOR_BG || *bg == SELECTION_INACTIVE_BG);
            if let Some(background) = background {
                let selection = selected_row(background == CURSOR_BG);
                for span in &mut line.spans {
                    span.style = span.style.patch(selection);
                }
                let padding = (width as usize).saturating_sub(line.width());
                line.push_span(Span::styled(" ".repeat(padding), selection));
                line.style = selection;
            }
            line
        })
        .collect()
}

/// Popup action badges without altering label text or spacing.
pub(crate) fn action_line(text: &'static str) -> Line<'static> {
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let Some(end) = rest[start..].find(']').map(|end| start + end + 1) else {
            break;
        };
        spans.push(Span::raw(&rest[..start]));
        spans.push(chip(&rest[start..end], PRIMARY));
        rest = &rest[end..];
    }
    spans.push(Span::styled(rest, muted()));
    Line::from(spans)
}

/// Background for a cell under the pointer. A cell that already carries a fill
/// keeps its hue and is lifted instead of replaced, because that fill pairs with
/// a dedicated foreground (a selected row, a chip) that a flat tint would hide.
pub(crate) fn hover_fill(bg: Color) -> Color {
    /// Headroom that reads as a distinct hover on a dark terminal without
    /// washing the label out against its own foreground.
    const LIFT: u8 = 0x20;
    match bg {
        Color::Reset => HOVER_BG,
        Color::Rgb(r, g, b) => Color::Rgb(
            r.saturating_add(LIFT),
            g.saturating_add(LIFT),
            b.saturating_add(LIFT),
        ),
        filled => filled,
    }
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
pub(crate) fn footer_line(hints: impl IntoIterator<Item = Span<'static>>) -> Line<'static> {
    let mut spans = Vec::new();
    for hint in hints {
        let text = hint.content;
        let mut rest = text.as_ref();
        // Preserve the legacy description-before-key wording and all whitespace.
        while let Some(start) = rest.find('[') {
            let Some(end) = rest[start..].find(']').map(|end| start + end + 1) else {
                break;
            };
            spans.push(Span::styled(
                rest[..start].to_owned(),
                Style::default().fg(FOOTER_DESC),
            ));
            spans.extend(
                key_hint(&rest[start..end], "")
                    .into_iter()
                    .map(|span| Span::styled(span.content.into_owned(), span.style)),
            );
            rest = &rest[end..];
        }
        spans.push(Span::styled(
            rest.to_owned(),
            Style::default().fg(FOOTER_DESC).patch(hint.style),
        ));
    }
    Line::from(spans).style(Style::default().fg(FOOTER_DESC).bg(FOOTER_BG))
}

/// Low-emphasis metadata style.
pub(crate) fn muted() -> Style {
    Style::default().fg(MUTED)
}

/// Semantic run state foreground.
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
pub(crate) fn diff_style(kind: DiffStatus) -> Style {
    Style::default().fg(match kind {
        DiffStatus::Added => SUCCESS,
        DiffStatus::Modified => WARNING,
        DiffStatus::Deleted => ERROR,
        DiffStatus::Unchanged => MUTED,
    })
}

/// Minimum vertical thumb length. Proportional geometry collapses to a single
/// row on very long content, which is hard to see and to grab.
const MIN_SCROLLBAR_THUMB: usize = 3;

/// Minimum horizontal thumb length, in cells. A horizontal bar spans more
/// cells than a vertical bar spans rows, so a proportional thumb still needs
/// room to read as a target.
const MIN_SCROLLBAR_THUMB_HORIZONTAL: usize = 4;

/// Axis a scrollbar runs along, which selects the extent it measures and paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScrollAxis {
    Vertical,
    Horizontal,
}

impl ScrollAxis {
    fn min_thumb(self) -> usize {
        match self {
            ScrollAxis::Vertical => MIN_SCROLLBAR_THUMB,
            ScrollAxis::Horizontal => MIN_SCROLLBAR_THUMB_HORIZONTAL,
        }
    }
}

/// Pointer state of a scrollbar, which selects the thumb fill; the track fill
/// never changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThumbState {
    Idle,
    Hovered,
    Grabbed,
}

/// Paints a scrollbar as solid cells, the way iCode's Textual bar paints
/// reverse-video blocks.
///
/// A vertical bar returns one line per row; a horizontal bar returns a single
/// line of `extent` cells, because it spans one row. Thumb and track are
/// filled with a background rather than a glyph so the bar stays solid
/// whatever font the terminal uses; `state` lifts the thumb fill only.
pub(crate) fn scrollbar_lines(
    total: usize,
    extent: usize,
    start: usize,
    axis: ScrollAxis,
    state: ThumbState,
) -> Vec<Line<'static>> {
    let Some((thumb_pos, thumb_len)) = scrollbar_thumb(total, extent, start, axis) else {
        return Vec::new();
    };
    let thumb = Style::default().bg(match state {
        ThumbState::Idle => SCROLL_THUMB,
        ThumbState::Hovered => SCROLL_THUMB_HOVER,
        ThumbState::Grabbed => SCROLL_THUMB_ACTIVE,
    });
    let track = Style::default().bg(SCROLL_TRACK_BG);
    let cell = |index: usize| {
        if (thumb_pos..thumb_pos + thumb_len).contains(&index) {
            thumb
        } else {
            track
        }
    };
    match axis {
        ScrollAxis::Vertical => (0..extent)
            .map(|index| Line::styled(" ", cell(index)))
            .collect(),
        ScrollAxis::Horizontal => {
            let spans = (0..extent)
                .map(|index| Span::styled(" ", cell(index)))
                .collect::<Vec<_>>();
            vec![Line::from(spans)]
        }
    }
}

/// Thumb geometry for pointer hit-testing. Shares one implementation with the
/// drawn bar so a grab lands on the cells the user sees.
pub(crate) fn scrollbar_thumb(
    total: usize,
    extent: usize,
    start: usize,
    axis: ScrollAxis,
) -> Option<(usize, usize)> {
    scrollbar_geometry(total, extent, start, axis.min_thumb())
}

/// Resolves the thumb position and length for a viewport, or `None` when the
/// content fits and no bar is needed.
fn scrollbar_geometry(
    total: usize,
    height: usize,
    start: usize,
    min_thumb: usize,
) -> Option<(usize, usize)> {
    if height == 0 || total <= height {
        return None;
    }
    // Keep at least one track row visible when the viewport allows it, so a
    // short panel does not render a full-height thumb.
    let min_thumb = min_thumb.min(height.saturating_sub(1).max(1));
    let thumb_len = height
        .saturating_mul(height)
        .div_ceil(total)
        .max(min_thumb)
        .clamp(1, height);
    let track = height - thumb_len;
    let scrollable = total - height;
    let thumb_pos = start.min(scrollable).saturating_mul(track) / scrollable;
    Some((thumb_pos, thumb_len))
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
    fn scrollbar_bounds_clamp_and_hide_when_content_fits() {
        assert!(scrollbar_lines(10, 0, 0, ScrollAxis::Vertical, ThumbState::Idle).is_empty());
        assert!(scrollbar_lines(4, 4, 0, ScrollAxis::Vertical, ThumbState::Idle).is_empty());
        assert_eq!(
            scrollbar_lines(10, 4, usize::MAX, ScrollAxis::Vertical, ThumbState::Idle),
            scrollbar_lines(10, 4, 6, ScrollAxis::Vertical, ThumbState::Idle)
        );
        assert_eq!(
            scrollbar_lines(10, 4, 0, ScrollAxis::Vertical, ThumbState::Idle)[0]
                .style
                .bg,
            Some(SCROLL_THUMB)
        );
        assert_eq!(
            scrollbar_lines(10, 4, 6, ScrollAxis::Vertical, ThumbState::Idle)[3]
                .style
                .bg,
            Some(SCROLL_THUMB)
        );
    }

    fn thumb_cells(lines: &[Line<'static>]) -> usize {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .filter(|span| span.style.bg.or(line.style.bg) == Some(SCROLL_THUMB))
                    .count()
            })
            .sum()
    }

    #[test]
    fn vertical_scrollbar_keeps_a_grabbable_thumb() {
        assert_eq!(
            thumb_cells(&scrollbar_lines(
                1000,
                20,
                0,
                ScrollAxis::Vertical,
                ThumbState::Idle
            )),
            MIN_SCROLLBAR_THUMB
        );
        let lengths = [40, 200, 2000].map(|total| {
            thumb_cells(&scrollbar_lines(
                total,
                20,
                0,
                ScrollAxis::Vertical,
                ThumbState::Idle,
            ))
        });
        assert!(lengths[0] >= lengths[1] && lengths[1] >= lengths[2]);
        assert!(lengths[2] >= MIN_SCROLLBAR_THUMB);
        // Tiny viewports still leave one track row instead of a full thumb.
        assert_eq!(
            thumb_cells(&scrollbar_lines(
                1000,
                2,
                0,
                ScrollAxis::Vertical,
                ThumbState::Idle
            )),
            1
        );
    }

    #[test]
    fn vertical_scrollbar_thumb_sticks_to_both_ends() {
        let top = scrollbar_lines(1000, 20, 0, ScrollAxis::Vertical, ThumbState::Idle);
        assert_eq!(top[0].style.bg, Some(SCROLL_THUMB));
        let bottom = scrollbar_lines(1000, 20, 980, ScrollAxis::Vertical, ThumbState::Idle);
        assert_eq!(bottom[0].style.bg, Some(SCROLL_TRACK_BG));
        assert_eq!(bottom[19].style.bg, Some(SCROLL_THUMB));
    }

    #[test]
    fn horizontal_scrollbar_paints_one_row_of_solid_cells() {
        let lines = scrollbar_lines(1000, 20, 0, ScrollAxis::Horizontal, ThumbState::Idle);
        assert_eq!(lines.len(), 1, "a horizontal bar spans one row");
        assert_eq!(lines[0].width(), 20);
        assert_eq!(
            lines[0]
                .spans
                .iter()
                .filter(|span| span.style.bg == Some(SCROLL_THUMB))
                .count(),
            MIN_SCROLLBAR_THUMB_HORIZONTAL
        );
        assert_eq!(lines[0].spans[0].style.bg, Some(SCROLL_THUMB));
    }

    #[test]
    fn scrollbar_paints_solid_cells_with_pointer_state() {
        let lines = scrollbar_lines(1000, 20, 0, ScrollAxis::Vertical, ThumbState::Idle);
        assert_eq!(lines.len(), 20);
        assert_eq!(lines[0].spans[0].content, " ");
        assert_eq!(lines[0].style.bg, Some(SCROLL_THUMB));
        assert_eq!(lines[19].style.bg, Some(SCROLL_TRACK_BG));

        // Only the thumb answers the pointer; the track stays quiet.
        let hovered = scrollbar_lines(1000, 20, 0, ScrollAxis::Vertical, ThumbState::Hovered);
        assert_eq!(hovered[0].style.bg, Some(SCROLL_THUMB_HOVER));
        assert_eq!(hovered[19].style.bg, Some(SCROLL_TRACK_BG));
        let grabbed = scrollbar_lines(1000, 20, 0, ScrollAxis::Vertical, ThumbState::Grabbed);
        assert_eq!(grabbed[0].style.bg, Some(SCROLL_THUMB_ACTIVE));

        let horizontal = scrollbar_lines(1000, 20, 0, ScrollAxis::Horizontal, ThumbState::Grabbed);
        assert_eq!(horizontal[0].spans[0].style.bg, Some(SCROLL_THUMB_ACTIVE));
    }

    #[test]
    fn both_axes_share_the_border_bar_geometry() {
        assert!(scrollbar_lines(4, 4, 0, ScrollAxis::Horizontal, ThumbState::Idle).is_empty());
        assert!(scrollbar_lines(10, 0, 0, ScrollAxis::Horizontal, ThumbState::Idle).is_empty());
        let lines = scrollbar_lines(1000, 20, 0, ScrollAxis::Vertical, ThumbState::Idle);
        assert_eq!(thumb_cells(&lines), MIN_SCROLLBAR_THUMB);
        let horizontal = scrollbar_lines(1000, 20, 0, ScrollAxis::Horizontal, ThumbState::Idle);
        assert_eq!(thumb_cells(&horizontal), MIN_SCROLLBAR_THUMB_HORIZONTAL);
    }

    #[test]
    fn test_footer_preserves_text_and_styles_only_key_as_primary() {
        let line = footer_line([Span::raw(" Save [Ctrl+s] ")]);
        assert_eq!(line.to_string(), " Save [Ctrl+s] ");
        assert_eq!(line.style.bg, Some(FOOTER_BG));
        let key = line
            .spans
            .iter()
            .find(|span| span.content == "[Ctrl+s]")
            .unwrap();
        assert_eq!(key.style.fg, Some(PRIMARY));
        assert!(key.style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(line.spans[0].style.fg, Some(FOOTER_DESC));
    }

    #[test]
    fn test_popup_selection_fills_metadata_and_trailing_cells() {
        let lines = popup_rows(
            vec![Line::from(vec![
                Span::styled("Selected", selected_row(true)),
                Span::styled(" meta", muted()),
            ])],
            20,
        );
        assert_eq!(lines[0].width(), 20);
        assert!(
            lines[0]
                .spans
                .iter()
                .all(|span| span.style.bg == Some(CURSOR_BG) && span.style.fg == Some(CURSOR_FG))
        );
    }

    #[test]
    fn test_page_color_budget() {
        const CAPS: &[(&str, &str, usize)] = &[
            ("ui.rs", include_str!("ui.rs"), 0),
            ("highlight.rs", include_str!("highlight.rs"), 0),
            ("mindmap.rs", include_str!("mindmap.rs"), 0),
            ("authoring_tab.rs", include_str!("authoring_tab.rs"), 0),
            ("test_points_tab.rs", include_str!("test_points_tab.rs"), 0),
            ("ui/chat.rs", include_str!("ui/chat.rs"), 0),
            ("markdown.rs", include_str!("markdown.rs"), 0),
        ];
        for &(path, source, cap) in CAPS {
            let count = source.matches("Color::").count();
            assert!(count <= cap, "{path}: {count} raw colors exceeds cap {cap}");
        }
    }
}
