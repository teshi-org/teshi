## 1. Phase 0: Style guide and theme foundation

- [x] 1.1 Write `doc/tui-style-guide.md` in English. It must cover the ten design rules, the full token table (existing chat tokens plus the new ones in design.md), the component helper catalog, the glyph vocabulary, do/don't examples, and the per-phase migration checklist.
- [x] 1.2 Add `.cursor/rules/tui-style.mdc` with `globs: crates/teshi-tui/**/*.rs`. The rule body should say: use `theme::` helpers instead of `Color::`; pick glyphs only from the guide; lower the color-budget caps for every file you migrate.
- [x] 1.3 Commit 1.1 and 1.2 as `docs: add TUI style guide derived from chrys`.
- [x] 1.4 Rename `crates/teshi-tui/src/chat_theme.rs` to `theme.rs`, then update `lib.rs`, `markdown.rs`, and `ui/chat.rs` to use the new path.
- [x] 1.5 Add the new tokens: `FOOTER_BG`, `FOOTER_DESC`, `TAB_ACTIVE_BG`, `TAB_INACTIVE`, `SELECTION_INACTIVE_BG`, `EDITOR_SELECTION_BG`, `CURRENT_LINE_BG`, `OVERLAY_BG`, `DIFF_ADDED_BG`, `DIFF_MODIFIED_BG`, `DIFF_DELETED_BG`, and `SYNTAX_STRING`, each with a `///` doc comment.
- [x] 1.6 Add the component helpers `panel_block`, `popup_block`, `selected_row`, `chip`, `key_hint`, `footer_line`, `muted`, `status_style`, and `diff_style`. Move `scrollbar_lines` out of `ui/chat.rs` and switch the chat panel to the shared helpers.
- [x] 1.7 Add unit tests for the helpers: rounded border type, border color for focused and unfocused panels, and selection fill style.
- [x] 1.8 Add the color-budget test. Use `include_str!` and a cap table set to the current `Color::` counts (`ui.rs`, `highlight.rs`, `mindmap.rs`, `authoring_tab.rs`, `test_points_tab.rs`, `ui/chat.rs`, `markdown.rs`). The test must fail when a count goes over its cap.
- [x] 1.9 Run fmt, clippy, and the workspace tests. Commit as `refactor(tui): promote chat palette to shared TUI theme`.

## 2. Phase 1: Global chrome

- [x] 2.1 Restyle the top tab bar. Inactive tabs use `TAB_INACTIVE`; the active tab is bold on `TAB_ACTIVE_BG`. Keep the `ratatui::Tabs` padding and divider constants unchanged.
- [x] 2.2 Replace the divider and version row with a `BORDER` `─` rule plus a `PRIMARY` `━` segment under the active tab's registered `ClickableRegion::Tab` rect. Keep the version label right-aligned in the muted color.
- [x] 2.3 Switch the footer helpers (`footer_pill`, `footer_hints`, `render_explore_footer`, `render_ai_footer`) to `key_hint` and `footer_line` on the `FOOTER_BG` strip, without changing any hint text. Style `status_message` in `WARNING` bold.
- [x] 2.4 Move these popups to `popup_block`, `selected_row`, and `chip`: auth, model list and form, session, quit, approval, ACP agent, agent profile, failure detail, external change, change summary, scenario dropdown, step keyword picker, and the requirements overlay. Then remove `popup_highlight_block`.
- [x] 2.5 Add a `TestBackend` test that clicks every tab, checks that the tab activates, and checks that the `━` segment lines up with that tab's title.
- [x] 2.6 Lower the `ui.rs` cap in the budget test, run the gates, install with `scripts/install-local.ps1`, check the result visually, and commit as `feat(tui): apply chrys style to tabs, footers, and popups`.

## 3. Phase 2: Explore tab and editor

- [x] 3.1 Switch `explore_block` to `panel_block`. Use `selected_row(focused)` for column selection, and remove `EXPLORE_SELECTED_*`, `HIGHLIGHT_*`, and `selected_style` once nothing references them.
- [x] 3.2 Replace the `STATUS_*` constants and `status_color` with `status_style`. Route `git_file_status_style`, `git_diff_status_style`, and the `DIFF_*_BG` constants through `diff_style` and the theme tokens.
- [x] 3.3 Switch the Explore horizontal scrollbars to the shared `scrollbar_lines` glyphs and colors.
- [x] 3.4 Editor: use `EDITOR_SELECTION_BG` for text selection, and use `CURRENT_LINE_BG` with `FG` for the preview cursor line (replacing `PREVIEW_CURSOR_*`). Diagnostics use `ERROR` for errors, `WARNING` for warnings, and the muted color for repair hints.
- [x] 3.5 Remap `highlight.rs::kind_to_style` to the Gherkin palette from design.md. Replace the `KEYWORD_*` constants in `ui.rs` with the same mapping, and update the foreground assertions in the `highlight.rs` tests.
- [x] 3.6 Lower the `ui.rs` cap and set the `highlight.rs` cap to zero, run the gates, check visually, and commit as `feat(tui): apply chrys style to Explore and editor`.

## 4. Phase 3: MindMap tab

- [x] 4.1 Switch `render_tree_panel` to `panel_block` and `selected_row`.
- [x] 4.2 Preserve the project-root/step-body trie and visible text. Map the root to `FG` bold, background-derived steps to `SECONDARY`, and scenario-derived steps via the source keyword's Gherkin palette, including continuation inheritance. Preserve explicit and relationship highlight precedence. Record the user-confirmed mapping in the style guide.
- [x] 4.3 Use `CURRENT_LINE_BG` for the selected line in `render_mindmap_scenario_preview`, and move the scenario dropdown and change summary to the theme if phase 1 has not already done so.
- [x] 4.4 Set the `mindmap.rs` cap to zero, lower the `ui.rs` cap, run the gates, check visually, and commit as `feat(tui): apply chrys style to MindMap`.

## 5. Phase 4: Requirements and Test Points tabs

- [ ] 5.1 Switch the blocks and selection in `render_requirements_*` and `render_test_points_*` to `panel_block` and `selected_row`.
- [ ] 5.2 Replace the review and resolution colors and the footer builders in `authoring_tab.rs` and `test_points_tab.rs` with theme roles and `footer_line`.
- [ ] 5.3 Render the Requirements editor `[BROWSE]` / `[INSERT]` mode labels as chips (BROWSE `PRIMARY`, INSERT `ACCENT`), keeping the hint text the same.
- [ ] 5.4 Set every cap except `theme.rs` to zero, then run the gates, check visually, and commit as `feat(tui): apply chrys style to Requirements and Test Points`.

## 6. Close-out

- [ ] 6.1 Confirm `crates/teshi-tui-runner/tests/teshi_self.rs` and every `TestBackend` text test pass unchanged.
- [ ] 6.2 Update `doc/tui-style-guide.md` with any decisions made during the phases (footer strip width, MindMap mapping) and screenshots or ASCII samples of each tab.
- [ ] 6.3 Run `openspec validate adopt-chrys-tui-style` and archive the change once all phases are merged.

## Implementation evidence (2026-10-14)

- Documentation commit: `571479a` (`docs: add TUI style guide derived from chrys`).
- Foundation implementation is present but uncommitted pending task 1.9 gates.
- Passed: `cargo fmt --all --check`, `git diff --check`, workspace Clippy with
  `--exclude teshi-web --locked --all-targets --all-features -- -D warnings`.
- Passed: `cargo test -p teshi-tui --locked` (355 passed, 1 ignored; 2 doctests).
- Passed: `openspec validate adopt-chrys-tui-style` (validation portion only;
  close-out task 6.3 is not complete).
- Workspace tests failed in `check_cli` (5 failures), `steps_validation_gate`
  (1 failure), and `validation_cli_bdd` (2 failures). Feature paths resolve to
  `C:\Users\lilin` rather than the isolated test directory. Live inspection found
  `C:\Users\lilin\.teshi`; `teshi-engine/src/daemon.rs::find_project_root` walks
  ancestors for `.teshi`, explaining the unexpected project root for temporary
  test projects under the user profile. No user data or project-root behavior
  was changed. Task 1.9 remains unchecked; subsequent phases have not started.
- Newly introduced unused theme items carry individual temporary dead-code
  allowances for staged migration. Remove each allowance when its surface uses
  the item; do not retain them after migration.

### Foundation gate recovery

- Isolating `TEMP` and `TMP` to a new directory directly under `D:\` made all
  15 tests in the three previously failing CLI suites pass.
- A parallel workspace run then hit the existing ACP shutdown timing assertion
  (drop must take less than 4 seconds). Its isolated rerun passed in 2.43 seconds.
- The complete workspace suite passed with isolated temporary directories and
  `-- --test-threads=1`; no tests were skipped beyond their existing ignore flags.
  No project discovery code or user configuration was changed.

### Global chrome implementation and installation gate

- Tasks 2.1–2.5 are implemented: registered-region tab underline, themed footer
  hints and status strip, rounded popup chrome, full-width option selection and
  action badges. All five tab click/underline assertions pass.
- The `ui.rs` color budget was lowered from 182 to 99. Consumed foundation items
  no longer carry their temporary dead-code allowances.
- Passed workspace Clippy and the complete workspace tests using the isolated
  temporary root and `-- --test-threads=1`.
- Unchanged real-PTY self-tests `english_tui_command_edits_and_saves` and
  `chinese_run_command_routes_to_tui_and_saves` passed against the newly built CLI.
- Installation through `scripts/install-local.ps1` built successfully but failed
  replacing `C:\Users\lilin\.cargo\bin\teshi.exe` with access denied (os error 5).
  Process 8900 was running that exact executable. It was not terminated.
- Task 2.6 remains unchecked: installation, visual inspection and phase commit
  are pending normal closure of the installed application. Later phases have
  not started. The existing runner change was independently committed as
  `d0d8bab`; it is not part of this style implementation.

- After the user closed the application, the local install script succeeded.
  Captured installed-binary PTY viewports for all five tabs and the model popup
  in an isolated fixture; reviewed the rendered model screenshot for active tab
  fill/underline, rounded purple popup, full-width selected option and footer.
  The agent shell inherits `NO_COLOR=1` and `TERM=dumb`; capture child overrides
  were limited to `NO_COLOR=""`, `TERM=xterm-256color`, `COLORTERM=truecolor`.
  Captures are under `D:\teshi-style-tests-e71789e671124e31b7ffe7d82d0a59d6`.
  Task 2.6 is now complete. Final added helper tests passed (358 unit tests,
  1 existing ignored test, 2 doctests) and workspace Clippy passed again.

### Explore and editor phase

- Rounded focused panels, full-width active/inactive selection, shared status
  and diff roles, shared scrollbar glyphs, editor selection/current-line tokens,
  diagnostics and Gherkin palette are implemented. Removed the old local color
  constants; the compatibility selection wrapper remains while later surfaces
  still call it, and delegates to the shared helper.
- `highlight.rs` cap is zero; `ui.rs` cap is 35.
- Final workspace Clippy and complete serialized workspace tests passed.
- Installed successfully and visually reviewed real PTY Explore and editor
  screenshots, including inactive row fills and Given/When/Then colors.

### MindMap phase

- The user approved preserving the actual root/step trie instead of adding
  nonexistent Feature/Scenario nodes. Design and guide now describe the actual
  source-location mapping and merged-node precedence.
- Added a regression for node IDs/text, merged source locations, background
  styling and continuation inheritance. MindMap raw-color cap is zero;
  the intermediate `ui.rs` cap is 28.
- Workspace tests and Clippy passed. Installed successfully after workspace
  subprocess tests had exited, then reviewed the expanded real-PTY MindMap
  and scenario preview screenshot with full-width selection/current-line fill.
- Requirements/Test Points implementation was prepared independently and is
  being applied after this phase to preserve the requested commit boundaries.
