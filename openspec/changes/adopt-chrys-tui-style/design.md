## Context

The teshi TUI lives in `crates/teshi-tui` (ratatui 0.29, crossterm 0.28). The AI chat area was already migrated to a chrys-style look in commit `58e0e79`:
- `src/chat_theme.rs` holds the palette.
- `src/markdown.rs` renders themed Markdown.
- `src/ui/chat.rs` renders the chat panel with rounded borders, left rails, and solid-fill selection.

Everything else still uses ad-hoc styling:
- About 176 raw `Color::` values in `ui.rs`, 14 in `highlight.rs`, 11 in `mindmap.rs`, and 10 each in `authoring_tab.rs` and `test_points_tab.rs`.
- Mostly plain borders.
- Yellow/cyan foreground-only selection.
- Bracketed footer "pills".

The reference is the chrys Textual app (`D:/dev/python/chrys`, default `chrys` theme in `src/chrys/app/tui/theme.py` and `chrys.tcss`). The work will be split across several agents, so the style has to be written down precisely enough that independent phases come out consistent.

## Goals / Non-Goals

**Goals:**
- Extract chrys's visual rules into a reusable style guide and a single theme module.
- Migrate every TUI surface to that theme in independently committable phases.
- Make raw-color regressions mechanically detectable.

**Non-Goals:**
- No layout, keybinding, or visible-text changes. Tab geometry and click regions stay the same.
- No user-selectable themes or 16-color ANSI fallback. Tokens are centralized, so either can be added later.
- No changes to the GPUI desktop and web shells (`teshi-ui`, `teshi-web`, `teshi-desktop`).
- No syntax highlighting for non-Gherkin code blocks.

## Decisions

### 1. Design rules extracted from chrys
These rules go into `doc/tui-style-guide.md` and are the acceptance bar for every phase:

1. **Quiet chrome, loud content.** Borders are gray (`BORDER`) unless the panel has focus (`PRIMARY`). Color is spent on content and state, not decoration.
2. **Semantic roles, not colors.** Code refers to these roles: primary (brand/focus), secondary (rules), accent (user/input/emphasis), success/warning/error (state), and muted (hints/metadata).
3. **Shape.** Borders are rounded. Groups use left rails (`▌`) rather than nested boxes. Header, status, and footer rows are each 1 line, and the layout stays dense.
4. **Selection is a fill.** Focused selection uses cursor bg `#8787D7` with fg `#303030` across the full row. Unfocused selection uses `#444444`. Hover reverses video.
5. **Chips.** Modes, states, and tags are a solid background with dark foreground (`CHIP_FG`).
6. **Footer.** Keys render in `PRIMARY` bold and descriptions in `#D0D0D0`, on a `#262626` strip.
7. **Tabs.** Inactive tabs are dimmed (`#B2B2B2`). The active tab is bold on `#413755` (primary at 25% over the chrys `#1C1C1C` base), with a `━` underline in `PRIMARY` on the row below.
8. **Glyph vocabulary.**
   - Status: `● ○ ◆ ✓ ✗`
   - Spinner: `◐◓◑◒`
   - Rail: `▌`
   - Scrollbar: track `│`, thumb `┃`
   - Bullets: `• ▪ ‣ ⭑ ◦`
   - Menu current item: `◦`
   - Checkboxes: `[*] [ ]`
9. **Spacing.** Panels have 1 cell of horizontal padding and a blank line between groups. Empty states use the existing hatched placeholder.
10. **Popups.** Draw `Clear`, then a rounded `PRIMARY` border, a `PRIMARY` bold title, and actions as chips.

Alternative considered: hand the next agents only the chrys source paths. Rejected because chrys is CSS-driven Textual, and translating it per surface would drift. One written ratatui-specific translation keeps the phases consistent.

### 2. One theme module with tokens plus component helpers
Rename `chat_theme.rs` to `theme.rs` and keep its existing tokens. The new tokens are:

| Token | Value | Use |
|---|---|---|
| `FOOTER_BG` | `#262626` | footer strip |
| `FOOTER_DESC` | `#D0D0D0` | footer hint descriptions |
| `TAB_ACTIVE_BG` | `#413755` | active tab fill |
| `TAB_INACTIVE` | `#B2B2B2` | inactive tabs, And/But keywords |
| `SELECTION_INACTIVE_BG` | `#444444` | unfocused selection fill |
| `EDITOR_SELECTION_BG` | `#5F5F87` | editor text selection |
| `CURRENT_LINE_BG` | `#303030` | preview cursor line |
| `OVERLAY_BG` | `#3A3A3A` | toast or overlay surfaces |
| `DIFF_ADDED_BG` | `#263E2C` | added-line tint |
| `DIFF_MODIFIED_BG` | `#3E3226` | modified-line tint |
| `DIFF_DELETED_BG` | `#3E2626` | deleted-line tint |
| `SYNTAX_STRING` | `#D7AF87` | Gherkin strings |

Component helpers, each with rustdoc:
- `panel_block(title, focused) -> Block`
- `popup_block(title) -> Block`
- `selected_row(focused) -> Style`
- `chip(label, bg) -> Span`
- `key_hint(key, desc) -> Vec<Span>` and `footer_line(hints) -> Line`
- `muted() -> Style`
- `status_style(RunStatus) -> Style`
- `diff_style(kind) -> Style`
- `scrollbar_lines(total, height, start) -> Vec<Line>`, moved out of `ui/chat.rs`

Helpers rather than tokens alone, because most of today's inconsistency comes from each surface recombining tokens differently, for example a focused border that is yellow and bold in one place and blue in another. Helpers carry the combination rules.

Alternative considered: a `Theme` struct passed through `App`. Rejected for now because the palette is static and there is no theme switching (a non-goal). Constants plus free functions are simpler, and they can be wrapped in a struct later without touching call sites.

### 3. Ratcheting color-budget test
A `#[cfg(test)]` test in `theme.rs` reads each TUI source file with `include_str!`, counts `Color::` occurrences, and compares them against a `const` cap table (file, max). It fails when a file goes over its cap or when a file in the table is missing.
- Phase 0 sets the caps to the current counts.
- Each later phase lowers the caps for the files it migrates.
- The final phase sets every cap to zero.

This makes the rule "no raw colors in pages" mechanical, and parallel agents cannot reintroduce raw colors in areas that are already migrated.

Alternative considered: a clippy `disallowed_types` or `disallowed_methods` lint. Rejected because `Color` variants are enum values, not methods, so a lint cannot target them precisely. A grep in CI was also rejected, because a unit test runs locally under the existing `cargo test` gate.

### 4. Tabs keep the ratatui `Tabs` widget
The tab bar keeps `ratatui::Tabs` with the same `TAB_PADDING_LEFT`, `TAB_PADDING_RIGHT`, and `TAB_DIVIDER`, because `register_tab_regions` mirrors that geometry for mouse hit-testing. Only `style` and `highlight_style` change. The underline row reads the active tab's `ClickableRegion::Tab` rect that was registered earlier in the same frame, so the underline cannot drift from the click target.

### 5. One Gherkin color mapping
`highlight.rs::kind_to_style` becomes the only Gherkin palette, and the `KEYWORD_*` constants in `ui.rs` are replaced by calls into it or by theme tokens, so the editor, the Explore Steps column, and the MindMap preview cannot diverge. The mapping:

| Kind | Style |
|---|---|
| Header | `FG` bold |
| Tag | `ACCENT` |
| Given | `PRIMARY` |
| When | `WARNING` |
| Then | `SUCCESS` |
| And/But | `TAB_INACTIVE` |
| Comment | `MUTED` |
| String | `SYNTAX_STRING` |
| Meta/DocString | `SECONDARY` |

### 6. Phase boundaries
Phases follow surface boundaries rather than file boundaries. `ui.rs` holds most surfaces, and its cap drops a little in each phase.

| Phase | Scope | Commit type |
|---|---|---|
| 0 | guide, Cursor rule, theme module, budget test | `docs:` then `refactor:` |
| 1 | global chrome (tabs, underline row, footers, status line, popups) | `feat(tui):` |
| 2 | Explore columns, run status, git diff, editor, Gherkin palette | `feat(tui):` |
| 3 | MindMap tree, node colors, preview | `feat(tui):` |
| 4 | Requirements and Test Points, all caps to zero | `feat(tui):` |

Phase 0 must land first. Phases 1 through 4 can be done in any order after it, but they touch overlapping parts of `ui.rs`, so they should land one at a time.

## Risks / Trade-offs

- [Truecolor assumption: RGB tokens look wrong on 256-color or 16-color terminals] → Same constraint as the shipped chat restyle. Centralized tokens make a later ANSI fallback a single-module change.
- [Text-matching tests break if hint wording changes] → Visible text is frozen by the spec. Agents grep `crates/teshi-tui-runner/tests/teshi_self.rs` and `ui.rs`/`app.rs` `TestBackend` tests before changing any string.
- [Tab underline or footer restyle desynchronizes hit-testing] → Keep the `Tabs` geometry constants, derive the underline from registered regions, and add a `TestBackend` test that clicks each tab.
- [Parallel agents conflict in `ui.rs`] → Phases are serialized by commit. The budget test catches raw colors added by a stale branch.
- [Light terminal themes reduce contrast for `#EEEEEE` text and dark fills] → Accepted. chrys targets dark terminals too, and light-theme support is a non-goal.
- [`include_str!` budget test depends on file paths] → The cap table lists the paths explicitly and the test fails on a missing file, so a rename forces a table update.

## Migration Plan

Each phase is a separate commit on `dev`, and rollback is reverting that commit. Before committing a phase:
- Run `cargo fmt --all`.
- Run `cargo clippy --workspace --exclude teshi-web --all-targets --all-features -- -D warnings`.
- Run `cargo test --workspace --exclude teshi-web --no-fail-fast`. The `teshi-cli` suites `check_cli`, `steps_validation_gate`, and `validation_cli_bdd` already fail on the baseline and are not regressions.
- Close any running `teshi`, install with `scripts/install-local.ps1`, and check the phase's surfaces by eye.

Commit messages are English Conventional Commits with no `Co-authored-by` trailer. Remove it if the tooling appends one.

## Open Questions

- Should the footer strip background (`FOOTER_BG`) span the full width or only behind hints? Default: full width, as in chrys.
- Mapping MindMap node categories (Feature, Scenario, Background) to `FG` bold, `PRIMARY`, and `SECONDARY` is a proposal. Confirm visually in Phase 3 and record the outcome in the style guide.
