## Why

The teshi TUI has no shared visual language: about 230 hardcoded `Color::` values are scattered across `ui.rs`, `highlight.rs`, `mindmap.rs`, `authoring_tab.rs`, and `test_points_tab.rs`, borders are mostly plain boxes, and selection, focus, and footer hints each use ad-hoc yellow/cyan styling. The AI chat area was recently restyled after the chrys TUI (purple palette, rounded borders, rail-grouped messages) and the result was well received; the rest of the TUI now looks inconsistent next to it. Codifying the chrys style as an explicit guide and theme module lets several agents migrate the remaining surfaces independently while staying consistent.

## What Changes

- Add a written TUI style guide (`doc/tui-style-guide.md`) that extracts the chrys design rules: quiet chrome with loud content, semantic color roles, rounded borders and left rails, solid-fill selection, chips, key-hint footer, tab bar with underline, a fixed glyph vocabulary, spacing, and popup conventions.
- Add a Cursor rule (`.cursor/rules/tui-style.mdc`) scoped to `crates/teshi-tui/**/*.rs` that points agents at the guide.
- Promote `crates/teshi-tui/src/chat_theme.rs` to a TUI-wide `theme.rs` with extra tokens (footer, tabs, selection, current line, overlay, diff tints, syntax string) and reusable component helpers (panel/popup blocks, selected-row style, chip, key hints, status and diff styles, scrollbar).
- Add a ratcheting test that caps `Color::` usage per source file; caps only go down and reach zero outside `theme.rs` by the final phase.
- Restyle the global chrome: tab bar and its underline row, footer key hints, status message line, and every popup/overlay.
- Restyle the Explore tab and feature editor, including run-status colors, git diff colors, selection, diagnostics, and Gherkin syntax highlighting remapped to the chrys palette.
- Restyle the MindMap tree and scenario preview, and the Requirements and Test Points tabs.
- Visible text, key bindings, layout geometry, and tab hit-testing stay unchanged; this is a visual-only change.

## Capabilities

### New Capabilities
- `tui-visual-style`: Visual consistency contract for the terminal UI: a single theme module as the source of colors and component styles, the chrys-derived design rules each surface must follow, and the guard that keeps raw colors out of page code.

### Modified Capabilities
<!-- None: existing TUI specs define behavior only, not visual styling. -->

## Impact

- Code: `crates/teshi-tui/src/{theme.rs (renamed from chat_theme.rs), ui.rs, ui/chat.rs, markdown.rs, highlight.rs, mindmap.rs, authoring_tab.rs, test_points_tab.rs}`.
- Tests: `highlight.rs` foreground-color assertions change with the new Gherkin palette; new `TestBackend` tests cover tab hit-testing and the tab underline. Text-based tests in `crates/teshi-tui-runner/tests/teshi_self.rs` must keep passing unchanged.
- Docs and tooling: new `doc/tui-style-guide.md` and `.cursor/rules/tui-style.mdc`.
- No dependency, API, CLI, or configuration changes. GPUI desktop/web shells are out of scope.
