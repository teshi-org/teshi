# TUI style guide

The ratatui TUI follows the default chrys palette. `src/theme.rs` in teshi-tui
is the source of truth. GPUI desktop and web shells are outside this guide.

## Ten design rules

1. Quiet chrome, prominent content: BORDER for unfocused panels, PRIMARY for focus.
2. Use semantic roles through theme helpers rather than constructing page colors.
3. Rounded borders and `▌` group rails; keep header/status/footer rows one line high.
4. Fill full selected rows with CURSOR_BG/CURSOR_FG when focused and SELECTION_INACTIVE_BG/FG otherwise. Hover reverses video.
5. Modes, states, and tags use solid chips with CHIP_FG text.
6. Footer keys use PRIMARY bold and descriptions FOOTER_DESC on a full-width FOOTER_BG strip.
7. Inactive tabs use TAB_INACTIVE. Active tabs use TAB_ACTIVE_BG and bold text, with a PRIMARY underline derived from the registered click region.
8. Use the documented glyph vocabulary below; document additions before use.
9. Preserve geometry; retain one-cell horizontal panel padding and blank lines between groups where the existing layout allows them. Retain hatched empty states.
10. Clear popups first, then draw rounded PRIMARY borders, PRIMARY bold titles, and action chips.

Never paint the terminal background globally. Fill only selections, chips, active
tabs, footers, code/quote blocks, current lines, overlays, and diff tints.
Preserve visible text, key bindings, tab padding, dividers, and click regions.

## Token table

| Token | RGB | Role |
|---|---|---|
| PRIMARY | #AF87FF | Brand, focus, headings, links |
| PRIMARY_MUTED | #875FAF | Quiet rails |
| SECONDARY | #5F5FAF | Rules, metadata syntax |
| ACCENT | #FF87D7 | User/input emphasis, tags |
| SUCCESS | #5FFF87 | Passed, added |
| WARNING | #FFAF5F | Running, modified, status messages |
| ERROR | #FF5F5F | Failed, deleted, diagnostics |
| FG | #EEEEEE | Default foreground |
| MUTED | #949494 | Hints, metadata, pending |
| BORDER | #5F5F5F | Unfocused borders, skipped |
| BLOCK_BG | #303030 | Code and quote surfaces |
| SCROLL_THUMB | #585858 | Scrollbar thumb |
| SCROLL_TRACK | #3A3A3A | Scrollbar track |
| CURSOR_BG | #8787D7 | Focused row fill |
| CURSOR_FG | #303030 | Focused row text |
| CHIP_FG | #1C1C1C | Chip text |
| FOOTER_BG | #262626 | Full-width footer strip |
| FOOTER_DESC | #D0D0D0 | Footer descriptions |
| TAB_ACTIVE_BG | #413755 | Active tab fill |
| TAB_INACTIVE | #B2B2B2 | Inactive tabs, And/But |
| SELECTION_INACTIVE_BG | #444444 | Inactive row fill |
| EDITOR_SELECTION_BG | #5F5F87 | Selected editor text |
| CURRENT_LINE_BG | #303030 | Preview cursor line |
| OVERLAY_BG | #3A3A3A | Overlay surface |
| DIFF_ADDED_BG | #263E2C | Added-line tint |
| DIFF_MODIFIED_BG | #3E3226 | Modified-line tint |
| DIFF_DELETED_BG | #3E2626 | Deleted-line tint |
| SYNTAX_STRING | #D7AF87 | Gherkin strings |

## Component helper catalog

| Helper | Contract |
|---|---|
| `panel_block(title, focused)` | Rounded semantic border and PRIMARY bold title |
| `popup_block(title)` | Focused panel chrome; caller draws Clear first |
| `selected_row(focused)` | Full-row fill style; apply to row widget |
| `chip(label, bg)` | Solid background, CHIP_FG bold text |
| `key_hint(key, desc)` | PRIMARY bold key, FOOTER_DESC description |
| `footer_line(hints)` | Hint line on FOOTER_BG; render paragraph across footer width |
| `muted()` | Metadata foreground |
| `status_style(status)` | Idle muted, running warning, passed success, failed error, skipped border |
| `diff_style(kind)` | Semantic diff foreground; apply matching tint for tinted rows |
| `scrollbar_lines(total, height, start)` | Vertical track/thumb with clamped position |

Gherkin: Header FG bold; Tag ACCENT; Given PRIMARY; When WARNING; Then SUCCESS;
And/But TAB_INACTIVE; Comment MUTED; String SYNTAX_STRING; Meta/DocString SECONDARY.
Preserve And/But inheritance. MindMap: Feature FG bold, Scenario PRIMARY,
Background SECONDARY, step keywords via the shared Gherkin mapping.

## Glyph vocabulary

| Purpose | Glyphs |
|---|---|
| Status | `● ○ ◆ ✓ ✗` |
| Spinner | `◐◓◑◒` |
| Rail | `▌` |
| Scrollbar | Track `│`, thumb `┃` |
| Bullets | `• ▪ ‣ ⭑ ◦` |
| Current menu item | `◦` |
| Checkboxes | `[*] [ ]` |
| Tab rule | `─`, active `━` |

Existing literal text and punctuation remain unchanged during migration.

## Do and don't

Do use `theme::panel_block(" Features ", focused)` and apply
`theme::selected_row(focused)` across the full row. Do use
`theme::chip("[INSERT]", theme::ACCENT)` and `theme::key_hint("s", "save")`.
Do not construct raw colors in pages, use foreground-only selection, paint
whole panels, or calculate tab underline widths independently of click regions.

## Per-phase migration checklist

- Phase 0: guide, Cursor rule, shared tokens/helpers, helper tests, baseline caps.
- Phase 1: tabs/underline, footers, popups, tab click/render test; lower ui cap.
- Phase 2: Explore panels, status/diffs, scrollbars, editor, shared Gherkin mapping; highlight cap zero; lower ui cap.
- Phase 3: MindMap panels, category mapping, preview; mindmap cap zero; lower ui cap.
- Phase 4: Requirements/Test Points panels, footer and chips; all page caps zero.
- Close-out: unchanged text and runner tests, tab samples, OpenSpec validation; archive after merge.

For every phase run formatting, workspace clippy and workspace tests excluding
the wasm-only teshi-web crate. Install with the dev-profile local install script
and inspect the affected surfaces visually. Record failed gates and visual
acceptance separately; text tests alone do not prove visual acceptance.
