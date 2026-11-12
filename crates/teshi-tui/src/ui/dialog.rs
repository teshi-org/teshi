//! Modal dialogs modeled on iCode's `ConfirmDialog`: a dimmed backdrop, a dark
//! padded card with a quiet border, and a centered row of solid buttons.
//!
//! [`Dialog`] centers a confirmation message; [`ListDialog`] shows selectable
//! rows that scroll with the selection. Callers own input handling and click
//! targets: rendering returns the drawn rectangles so each caller registers
//! its own clickable regions for them.

use crate::theme::{self, ButtonTone};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

/// Width tiers, as in iCode, so dialogs share a few sizes instead of one per caller.
const WIDTH_TIERS: [u16; 4] = [43, 64, 80, 100];
/// Narrowest button, so short labels such as `[No]` stay easy to hit.
const MIN_BUTTON_WIDTH: u16 = 16;
/// Blank cells on each side of a button label.
const BUTTON_LABEL_PAD: u16 = 2;
/// Blank cells on each side of a label when many buttons share one row.
const DENSE_LABEL_PAD: u16 = 1;
/// More buttons than this share one row as compact one-row buttons, so a
/// dialog with several secondary actions does not grow to the widest tier.
const MAX_TALL_BUTTONS: usize = 3;
/// Cells between adjacent buttons.
const BUTTON_GAP: u16 = 2;
/// Blank cells between the border and the content on each side.
const PAD_X: u16 = 2;
/// Padding kept when the terminal is too narrow for the content.
const MIN_PAD_X: u16 = 1;
/// Rows of a full button: a half-block top edge, the label, a half-block bottom edge.
const TALL_BUTTON_ROWS: u16 = 3;
/// Rows outside the content and buttons: two borders, the top margin and the
/// gap above the buttons.
const CHROME_ROWS: u16 = 4;
/// Tallest list before it scrolls, so a long list does not cover the screen.
const MAX_LIST_ROWS: u16 = 14;

/// One dialog button.
pub(super) struct DialogButton<'a> {
    /// Visible label, including its key hint, e.g. `Delete [Enter/y]`.
    pub label: &'a str,
    pub tone: ButtonTone,
    /// Whether Enter or the current arrow selection activates this button.
    pub focused: bool,
}

/// A centered confirmation dialog.
pub(super) struct Dialog<'a> {
    pub title: &'a str,
    /// Draws a warning border for destructive or irreversible decisions.
    pub warning: bool,
    /// Message lines, centered and wrapped to the dialog width.
    pub body: Vec<Line<'a>>,
    pub buttons: Vec<DialogButton<'a>>,
}

impl Dialog<'_> {
    /// Dims `area`, draws the dialog centered in it, and returns one rectangle
    /// per button, in order. A button that does not fit gets an empty rectangle.
    pub(super) fn render(self, frame: &mut Frame<'_>, area: Rect) -> Vec<Rect> {
        let body_w = max_width(&self.body);
        let (width, pad_x) = dialog_width(self.title, body_w, &self.buttons, area.width);
        let content_w = content_width(width, pad_x);
        let body_rows = wrapped_rows(&self.body, content_w);
        let fixed = CHROME_ROWS + body_rows;
        let plan = ButtonPlan::new(&self.buttons, content_w, area.height.saturating_sub(fixed));
        let panel = centered(area, width, fixed + plan.rows);
        let content = draw_chrome(frame, area, panel, self.title, self.warning, pad_x);

        let body_area = Rect::new(content.x, content.y, content.width, body_rows);
        frame.render_widget(
            Paragraph::new(self.body)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: false }),
            body_area.intersection(panel),
        );
        draw_buttons(frame, &self.buttons, &plan, content, panel)
    }
}

/// A dialog listing selectable rows above optional notes and buttons.
pub(super) struct ListDialog<'a> {
    pub title: &'a str,
    /// One line per option; style the selected one with `theme::selected_row`.
    /// Its fill is extended across the dialog width.
    pub rows: Vec<Line<'a>>,
    /// Index of the highlighted row, kept visible when the list scrolls.
    pub selected: usize,
    /// Muted explanations or an empty-state message shown below the list.
    pub notes: Vec<Line<'a>>,
    pub buttons: Vec<DialogButton<'a>>,
}

/// Click targets drawn by [`ListDialog::render`].
pub(super) struct ListTargets {
    /// Visible rows as `(row index, rect)`.
    pub rows: Vec<(usize, Rect)>,
    /// One rectangle per button, in order; empty when the button did not fit.
    pub buttons: Vec<Rect>,
}

impl ListDialog<'_> {
    /// Dims `area`, draws the dialog centered in it, and returns its targets.
    pub(super) fn render(self, frame: &mut Frame<'_>, area: Rect) -> ListTargets {
        let text_w = max_width(&self.rows).max(max_width(&self.notes));
        let (width, pad_x) = dialog_width(self.title, text_w, &self.buttons, area.width);
        let content_w = content_width(width, pad_x);
        let notes_rows = wrapped_rows(&self.notes, content_w);
        // A blank row separates the notes from the list above them.
        let notes_block = if notes_rows == 0 { 0 } else { notes_rows + 1 };
        let fixed = CHROME_ROWS + notes_block;
        let wanted = (self.rows.len() as u16).min(MAX_LIST_ROWS);
        let plan = ButtonPlan::new(
            &self.buttons,
            content_w,
            area.height.saturating_sub(fixed + wanted),
        );
        let visible = wanted.min(area.height.saturating_sub(fixed + plan.rows));
        let panel = centered(area, width, fixed + visible + plan.rows);
        let content = draw_chrome(frame, area, panel, self.title, false, pad_x);

        // Scroll just enough to keep the selection on the last visible row.
        let start = self
            .selected
            .saturating_sub(usize::from(visible.saturating_sub(1)));
        let mut rows = Vec::with_capacity(usize::from(visible));
        for (offset, line) in self
            .rows
            .into_iter()
            .enumerate()
            .skip(start)
            .take(usize::from(visible))
        {
            let rect = Rect::new(
                content.x,
                content.y + (offset - start) as u16,
                content.width,
                1,
            )
            .intersection(panel);
            frame.render_widget(
                Paragraph::new(theme::popup_rows(vec![line], content.width)),
                rect,
            );
            rows.push((offset, rect));
        }
        if notes_rows > 0 {
            let notes = Rect::new(
                content.x,
                content.y + visible + 1,
                content.width,
                notes_rows,
            );
            frame.render_widget(
                Paragraph::new(self.notes).wrap(Wrap { trim: false }),
                notes.intersection(panel),
            );
        }
        let buttons = draw_buttons(frame, &self.buttons, &plan, content, panel);
        ListTargets { rows, buttons }
    }
}

/// One single-line text field of a [`FormDialog`].
pub(super) struct FormField<'a> {
    pub label: &'a str,
    /// Text before the caret.
    pub before: &'a str,
    /// Text from the caret to the end.
    pub after: &'a str,
    /// Whether typing goes to this field; only it draws the caret.
    pub focused: bool,
}

/// A dialog of labeled text fields above optional notes and buttons.
pub(super) struct FormDialog<'a> {
    pub title: &'a str,
    pub fields: Vec<FormField<'a>>,
    /// Live feedback and help shown below the fields, left-aligned.
    pub notes: Vec<Line<'a>>,
    pub buttons: Vec<DialogButton<'a>>,
}

/// Narrowest input bar, so short values still leave room to type.
const MIN_INPUT_WIDTH: u16 = 36;

impl FormDialog<'_> {
    /// Dims `area`, draws the form centered in it, and returns one rectangle
    /// per button, in order.
    pub(super) fn render(self, frame: &mut Frame<'_>, area: Rect) -> Vec<Rect> {
        let label_w = self
            .fields
            .iter()
            .map(|field| field.label.width() as u16)
            .max()
            .unwrap_or(0);
        let value_w = self
            .fields
            .iter()
            .map(|field| (field.before.width() + field.after.width()) as u16 + 1)
            .max()
            .unwrap_or(0)
            .max(MIN_INPUT_WIDTH);
        let text_w = (label_w + 1 + value_w).max(max_width(&self.notes));
        let (width, pad_x) = dialog_width(self.title, text_w, &self.buttons, area.width);
        let content_w = content_width(width, pad_x);
        let field_rows = self.fields.len() as u16;
        let notes_rows = wrapped_rows(&self.notes, content_w);
        // A blank row separates the notes from the fields above them.
        let notes_block = if notes_rows == 0 { 0 } else { notes_rows + 1 };
        let fixed = CHROME_ROWS + field_rows + notes_block;
        let plan = ButtonPlan::new(&self.buttons, content_w, area.height.saturating_sub(fixed));
        let panel = centered(area, width, fixed + plan.rows);
        let content = draw_chrome(frame, area, panel, self.title, false, pad_x);

        for (row, field) in self.fields.iter().enumerate() {
            let rect = Rect::new(content.x, content.y + row as u16, content.width, 1);
            frame.render_widget(
                Paragraph::new(field_line(field, label_w, content.width)),
                rect.intersection(panel),
            );
        }
        if notes_rows > 0 {
            let notes = Rect::new(
                content.x,
                content.y + field_rows + 1,
                content.width,
                notes_rows,
            );
            frame.render_widget(
                Paragraph::new(self.notes).wrap(Wrap { trim: false }),
                notes.intersection(panel),
            );
        }
        draw_buttons(frame, &self.buttons, &plan, content, panel)
    }
}

/// A field row: the label column, then an input bar filling the rest of the
/// row. Long text scrolls so the caret stays inside the bar.
fn field_line<'a>(field: &FormField<'a>, label_w: u16, width: u16) -> Line<'a> {
    let label_style = if field.focused {
        Style::default()
            .fg(theme::PRIMARY)
            .add_modifier(Modifier::BOLD)
    } else {
        theme::muted()
    };
    let bar_w = usize::from(width.saturating_sub(label_w + 1));
    let bar = theme::input_style(field.focused);
    let caret_w = usize::from(field.focused);
    // Drop leading characters until the text before the caret, plus the
    // caret cell itself, fits the bar.
    let mut before = field.before;
    while before.width() + caret_w > bar_w {
        let mut chars = before.chars();
        if chars.next().is_none() {
            break;
        }
        before = chars.as_str();
    }
    let pad = usize::from(label_w).saturating_sub(field.label.width());
    let mut spans = vec![
        Span::styled(field.label, label_style),
        Span::raw(" ".repeat(pad + 1)),
        Span::styled(before, bar),
    ];
    let mut used = before.width();
    let mut after = field.after.chars();
    if field.focused {
        let under = after.next().map_or_else(|| " ".to_string(), String::from);
        used += under.width();
        spans.push(Span::styled(
            under,
            Style::default().fg(theme::CURSOR_FG).bg(theme::CURSOR_BG),
        ));
    }
    let rest = after.as_str();
    used += rest.width();
    spans.push(Span::styled(rest, bar));
    spans.push(Span::styled(" ".repeat(bar_w.saturating_sub(used)), bar));
    Line::from(spans)
}

/// How the button row fits the dialog.
struct ButtonPlan {
    /// Buttons that cannot share a row are stacked one per row.
    stacked: bool,
    /// Many buttons use compact labels and one row.
    dense: bool,
    /// Rows of each button when they share one row.
    height: u16,
    /// Rows used by the whole button block.
    rows: u16,
}

impl ButtonPlan {
    /// Plans the buttons for `content_w` columns and `spare` rows left over
    /// after the rest of the dialog.
    fn new(buttons: &[DialogButton<'_>], content_w: u16, spare: u16) -> Self {
        let dense = buttons.len() > MAX_TALL_BUTTONS;
        let count = buttons.len() as u16;
        if buttons_width(buttons, dense) > content_w {
            return Self {
                stacked: true,
                dense,
                height: 1,
                rows: count,
            };
        }
        // Tall buttons are the iCode look; short terminals fall back to one row.
        let height = if !dense && spare >= TALL_BUTTON_ROWS {
            TALL_BUTTON_ROWS
        } else {
            1
        };
        Self {
            stacked: false,
            dense,
            height,
            rows: if buttons.is_empty() { 0 } else { height },
        }
    }
}

/// Dialog width and horizontal padding for content `text_w` columns wide.
fn dialog_width(title: &str, text_w: u16, buttons: &[DialogButton<'_>], area_w: u16) -> (u16, u16) {
    let chrome_w = 2 + 2 * PAD_X;
    let dense = buttons.len() > MAX_TALL_BUTTONS;
    let needed = [
        title.trim().width() as u16 + 6,
        text_w + chrome_w,
        buttons_width(buttons, dense) + chrome_w,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    let tier = WIDTH_TIERS
        .into_iter()
        .find(|tier| *tier >= needed)
        .unwrap_or(WIDTH_TIERS[WIDTH_TIERS.len() - 1]);
    // Leave a visible backdrop margin, as iCode caps dialogs at 90%, unless
    // the content would then wrap: a narrow terminal spends the margin and
    // the extra padding on the content instead.
    let ninety_percent = (u32::from(area_w) * 9 / 10) as u16;
    let roomy = needed <= ninety_percent;
    let cap = if roomy { ninety_percent } else { area_w };
    let width = tier.min(cap).max(8).min(area_w);
    (width, if roomy { PAD_X } else { MIN_PAD_X })
}

fn content_width(width: u16, pad_x: u16) -> u16 {
    width.saturating_sub(2 + 2 * pad_x).max(1)
}

fn max_width(lines: &[Line<'_>]) -> u16 {
    lines.iter().map(Line::width).max().unwrap_or(0) as u16
}

fn wrapped_rows(lines: &[Line<'_>], width: u16) -> u16 {
    if lines.is_empty() {
        return 0;
    }
    Paragraph::new(lines.to_vec())
        .wrap(Wrap { trim: false })
        .line_count(width) as u16
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let height = height.min(area.height);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

/// Veils `area`, draws the card for `panel`, and returns its content area
/// below the top margin, between the side paddings.
fn draw_chrome(
    frame: &mut Frame<'_>,
    area: Rect,
    panel: Rect,
    title: &str,
    warning: bool,
    pad_x: u16,
) -> Rect {
    dim_backdrop(frame, area);
    frame.render_widget(Clear, panel);
    let title = format!(" {} ", title.trim());
    frame.render_widget(theme::dialog_block(title, warning), panel);
    Rect::new(
        panel.x + 1 + pad_x,
        panel.y + 2,
        content_width(panel.width, pad_x),
        panel.height.saturating_sub(CHROME_ROWS),
    )
}

/// Draws the button block at the bottom of `panel` and returns one rect per button.
fn draw_buttons(
    frame: &mut Frame<'_>,
    buttons: &[DialogButton<'_>],
    plan: &ButtonPlan,
    content: Rect,
    panel: Rect,
) -> Vec<Rect> {
    let y = panel.bottom().saturating_sub(1 + plan.rows);
    if plan.stacked {
        return buttons
            .iter()
            .enumerate()
            .map(|(index, button)| {
                let rect = Rect::new(content.x, y + index as u16, content.width, 1);
                draw_button(frame, button, rect.intersection(panel))
            })
            .collect();
    }
    let row_w = buttons_width(buttons, plan.dense);
    let mut x = content.x + content.width.saturating_sub(row_w) / 2;
    buttons
        .iter()
        .map(|button| {
            let width = button_width(button, plan.dense);
            let rect = Rect::new(x, y, width, plan.height);
            x += width + BUTTON_GAP;
            draw_button(frame, button, rect.intersection(panel))
        })
        .collect()
}

fn button_width(button: &DialogButton<'_>, dense: bool) -> u16 {
    let label = button.label.width() as u16;
    if dense {
        label + 2 * DENSE_LABEL_PAD
    } else {
        (label + 2 * BUTTON_LABEL_PAD).max(MIN_BUTTON_WIDTH)
    }
}

fn buttons_width(buttons: &[DialogButton<'_>], dense: bool) -> u16 {
    let labels: u16 = buttons.iter().map(|b| button_width(b, dense)).sum();
    labels + BUTTON_GAP * (buttons.len() as u16).saturating_sub(1)
}

/// Draws a solid button. A three-row button draws half-block edges in the
/// fill color so it reads as a raised pill rather than a flat bar.
fn draw_button(frame: &mut Frame<'_>, button: &DialogButton<'_>, rect: Rect) -> Rect {
    if rect.is_empty() {
        return rect;
    }
    let style = theme::button_style(button.tone, button.focused);
    let width = usize::from(rect.width);
    let label_w = button.label.width().min(width);
    let left = (width - label_w) / 2;
    let label = Line::from(vec![
        Span::raw(" ".repeat(left)),
        Span::raw(button.label),
        Span::raw(" ".repeat(width.saturating_sub(left + label_w))),
    ])
    .style(style);
    let lines = if rect.height >= TALL_BUTTON_ROWS {
        let edge = Style::default()
            .fg(theme::button_fill(button.tone, button.focused))
            .bg(theme::DIALOG_BG);
        vec![
            Line::styled("▄".repeat(width), edge),
            label,
            Line::styled("▀".repeat(width), edge),
        ]
    } else {
        vec![label]
    };
    frame.render_widget(Paragraph::new(lines), rect);
    rect
}

/// Veils everything already drawn in `area` so the dialog is the only bright
/// surface. Default backgrounds stay the terminal's own.
fn dim_backdrop(frame: &mut Frame<'_>, area: Rect) {
    let buffer = frame.buffer_mut();
    let area = area.intersection(buffer.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buffer[(x, y)];
            match theme::backdrop_color(cell.fg, true) {
                Some(fg) => cell.fg = fg,
                // Palette colors cannot be blended; let the terminal dim them.
                None => cell.modifier.insert(Modifier::DIM),
            }
            if let Some(bg) = theme::backdrop_color(cell.bg, false) {
                cell.bg = bg;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn render(width: u16, height: u16, dialog: Dialog<'_>) -> (Terminal<TestBackend>, Vec<Rect>) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut rects = Vec::new();
        terminal
            .draw(|frame| rects = dialog.render(frame, frame.area()))
            .unwrap();
        (terminal, rects)
    }

    fn render_list(
        width: u16,
        height: u16,
        dialog: ListDialog<'_>,
    ) -> (Terminal<TestBackend>, ListTargets) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut targets = None;
        terminal
            .draw(|frame| targets = Some(dialog.render(frame, frame.area())))
            .unwrap();
        (terminal, targets.unwrap())
    }

    fn button(label: &str, tone: ButtonTone, focused: bool) -> DialogButton<'_> {
        DialogButton {
            label,
            tone,
            focused,
        }
    }

    fn sample<'a>() -> Dialog<'a> {
        Dialog {
            title: "Confirm deletion",
            warning: true,
            body: vec![Line::raw("Delete Agent 1?")],
            buttons: vec![
                button("Delete [Enter/y]", ButtonTone::Danger, true),
                button("Cancel [Esc/n]", ButtonTone::Warning, false),
            ],
        }
    }

    fn list(count: usize, selected: usize) -> ListDialog<'static> {
        ListDialog {
            title: " Filter iteration ",
            rows: (0..count)
                .map(|index| {
                    let style = if index == selected {
                        theme::selected_row(true)
                    } else {
                        Style::default()
                    };
                    Line::styled(format!(" Option {index}"), style)
                })
                .collect(),
            selected,
            notes: vec![Line::styled("Pick one.", theme::muted())],
            buttons: vec![
                button("Select [Enter]", ButtonTone::Primary, true),
                button("Close [Esc]", ButtonTone::Warning, false),
            ],
        }
    }

    fn row_text(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn test_dialog_centers_tall_buttons_and_returns_their_rects() {
        let (terminal, rects) = render(100, 30, sample());
        assert_eq!(rects.len(), 2, "one rect per button");
        let (delete, cancel) = (rects[0], rects[1]);
        assert_eq!(
            delete.height, TALL_BUTTON_ROWS,
            "roomy screens get tall buttons"
        );
        assert_eq!(delete.y, cancel.y, "buttons share one row");
        assert_eq!(cancel.x, delete.right() + BUTTON_GAP);
        let label_row = row_text(&terminal, delete.y + 1);
        assert!(label_row.contains("Delete [Enter/y]"), "{label_row}");
        assert!(label_row.contains("Cancel [Esc/n]"), "{label_row}");
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(delete.x, delete.y)].symbol(), "▄");
        assert_eq!(
            buffer[(delete.x, delete.y + 1)].bg,
            theme::button_fill(ButtonTone::Danger, true)
        );
        assert_eq!(
            buffer[(cancel.x, cancel.y + 1)].bg,
            theme::button_fill(ButtonTone::Warning, false)
        );
        let left_gap = delete.x;
        let right_gap = buffer.area.width - cancel.right();
        assert!(left_gap.abs_diff(right_gap) <= 1, "row is centered");
    }

    #[test]
    fn test_dialog_uses_width_tiers_and_warning_border() {
        let (terminal, rects) = render(100, 30, sample());
        let buffer = terminal.backend().buffer();
        // Border, top margin, one message row, gap, then the buttons.
        let top = rects[0].y - 4;
        let left = (0..100)
            .find(|x| buffer[(*x, top)].symbol() == "╭")
            .expect("dialog top-left corner");
        let right = (0..100)
            .find(|x| buffer[(*x, top)].symbol() == "╮")
            .expect("dialog top-right corner");
        assert_eq!(right - left + 1, 64, "the button row selects the 64 tier");
        assert_eq!(buffer[(left, top)].fg, theme::WARNING);
        assert!(row_text(&terminal, top).contains(" Confirm deletion "));
    }

    #[test]
    fn test_dialog_dims_the_backdrop_without_painting_default_background() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Paragraph::new("behind").style(Style::default().fg(theme::FG)),
                    frame.area(),
                );
                sample().render(frame, frame.area());
            })
            .unwrap();
        let corner = &terminal.backend().buffer()[(0, 0)];
        assert_eq!(corner.symbol(), "b");
        assert_ne!(corner.fg, theme::FG, "covered text is veiled");
        assert_eq!(corner.bg, ratatui::style::Color::Reset);
    }

    #[test]
    fn test_narrow_dialog_spends_margin_and_padding_on_the_message() {
        let error = "Could not delete: cannot delete the last remaining";
        let mut dialog = sample();
        dialog.body.push(Line::raw(error));
        let (terminal, _) = render(55, 16, dialog);
        let screen = (0..16).map(|y| row_text(&terminal, y)).collect::<Vec<_>>();
        assert!(
            screen.iter().any(|row| row.contains(error)),
            "the phrase stays on one row:\n{}",
            screen.join("\n")
        );
        assert!(
            screen.iter().any(|row| row.starts_with('╭')),
            "the dialog takes the full width"
        );
    }

    #[test]
    fn test_dialog_falls_back_to_compact_and_stacked_buttons() {
        let (_, rects) = render(100, 7, sample());
        assert_eq!(rects[0].height, 1, "short screens get one-row buttons");
        assert_eq!(rects[0].y, rects[1].y);

        let (_, rects) = render(30, 20, sample());
        assert_eq!(rects[0].height, 1);
        assert_eq!(
            rects[1].y,
            rects[0].y + 1,
            "narrow screens stack buttons one per row"
        );
    }

    #[test]
    fn test_list_dialog_fills_the_selected_row_and_returns_row_targets() {
        let (terminal, targets) = render_list(100, 30, list(3, 1));
        assert_eq!(
            targets
                .rows
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        let (_, selected) = targets.rows[1];
        let buffer = terminal.backend().buffer();
        for x in selected.left()..selected.right() {
            assert_eq!(
                buffer[(x, selected.y)].bg,
                theme::CURSOR_BG,
                "the selection spans the row"
            );
        }
        assert!(row_text(&terminal, selected.y).contains("Option 1"));
        let notes_y = targets.rows[2].1.y + 2;
        assert!(row_text(&terminal, notes_y).contains("Pick one."));
        assert_eq!(targets.buttons.len(), 2);
        assert_eq!(targets.buttons[0].height, TALL_BUTTON_ROWS);
    }

    #[test]
    fn test_list_dialog_scrolls_to_keep_the_selection_visible() {
        let (_, targets) = render_list(100, 40, list(30, 25));
        assert_eq!(targets.rows.len(), usize::from(MAX_LIST_ROWS));
        assert_eq!(
            targets.rows.last().map(|(index, _)| *index),
            Some(25),
            "the selection is the last visible row"
        );

        let (_, targets) = render_list(100, 12, list(30, 0));
        assert_eq!(targets.rows.first().map(|(index, _)| *index), Some(0));
        assert!(
            targets.rows.len() < usize::from(MAX_LIST_ROWS),
            "short screens shorten the list"
        );
    }

    fn form<'a>(name: &'a str, condition: &'a str) -> FormDialog<'a> {
        FormDialog {
            title: "New saved query",
            fields: vec![
                FormField {
                    label: "Name",
                    before: name,
                    after: "",
                    focused: false,
                },
                FormField {
                    label: "Condition",
                    before: condition,
                    after: "",
                    focused: true,
                },
            ],
            notes: vec![Line::styled("Fields: status, priority", theme::muted())],
            buttons: vec![
                button("Save [Enter]", ButtonTone::Primary, true),
                button("Cancel [Esc]", ButtonTone::Warning, false),
            ],
        }
    }

    fn find(terminal: &Terminal<TestBackend>, text: &str) -> (u16, u16) {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .find_map(|y| {
                let row = row_text(terminal, y);
                row.find(text)
                    .map(|byte| (row[..byte].chars().count() as u16, y))
            })
            .unwrap_or_else(|| panic!("{text:?} not on screen"))
    }

    #[test]
    fn test_form_aligns_inputs_and_draws_the_caret_on_the_focused_field() {
        let (terminal, rects) = {
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            let mut rects = Vec::new();
            terminal
                .draw(|frame| rects = form("Ready", "status=planned").render(frame, frame.area()))
                .unwrap();
            (terminal, rects)
        };
        let (name_x, name_y) = find(&terminal, "Ready");
        let (condition_x, condition_y) = find(&terminal, "status=planned");
        assert_eq!(name_x, condition_x, "inputs share one column");
        assert_eq!(condition_y, name_y + 1);
        let buffer = terminal.backend().buffer();
        let caret = &buffer[(condition_x + "status=planned".len() as u16, condition_y)];
        assert_eq!(caret.bg, theme::CURSOR_BG, "the focused field has a caret");
        assert_eq!(
            buffer[(name_x, name_y)].bg,
            theme::input_style(false).bg.unwrap()
        );
        assert_eq!(
            buffer[(condition_x, condition_y)].bg,
            theme::input_style(true).bg.unwrap()
        );
        assert_eq!(rects.len(), 2);
    }

    #[test]
    fn test_form_scrolls_long_text_to_keep_the_caret_visible() {
        let long = "x".repeat(150);
        let condition = format!("{long}END");
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| {
                form("Ready", &condition).render(frame, frame.area());
            })
            .unwrap();
        let (end_x, end_y) = find(&terminal, "END");
        let caret = &terminal.backend().buffer()[(end_x + 3, end_y)];
        assert_eq!(caret.bg, theme::CURSOR_BG, "the caret follows the tail");
    }

    #[test]
    fn test_many_buttons_share_one_dense_row() {
        let mut dialog = list(2, 0);
        dialog.buttons = vec![
            button("Toggle [Space]", ButtonTone::Neutral, false),
            button("Color [c]", ButtonTone::Neutral, false),
            button("Front [f]", ButtonTone::Neutral, false),
            button("Save [Enter]", ButtonTone::Primary, true),
            button("Cancel [Esc]", ButtonTone::Warning, false),
        ];
        let (terminal, targets) = render_list(100, 30, dialog);
        let rects = targets.buttons;
        assert!(
            rects
                .iter()
                .all(|rect| rect.height == 1 && rect.y == rects[0].y)
        );
        assert_eq!(rects[0].width, "Toggle [Space]".width() as u16 + 2);
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(rects[0].x + 1, rects[0].y)].fg,
            theme::FG,
            "neutral labels stay readable on gray"
        );
    }
}
