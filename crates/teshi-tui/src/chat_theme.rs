//! Color palette for the AI chat surfaces and their Markdown rendering.
//!
//! Values follow the default `chrys` theme (a Dracula-inspired purple palette).
//! The terminal background is intentionally never painted so the chat blends
//! into the user's own terminal theme; only block surfaces use `BLOCK_BG`.

use ratatui::style::Color;

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
