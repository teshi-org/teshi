# Outstanding requirement implementation record

Subsequent scoped testing is recorded in
[requirements-testing-2026-10-27.md](requirements-testing-2026-10-27.md).
The authored-only and not-executed statements below describe the original delivery
checkpoint; consult that testing record for current evidence and remaining gaps.

Scope: doc-20 through doc-36 (17 requirements); doc-1 is an audit index.
Source: live user requirement library read on 2026-10-27, baseline f271c4a.
The global library, its statuses, previous evidence and running applications were not modified.
No commits, pushes, installation or application launches were performed.

## Decisions

- doc-24: new default names use UTC wall time with microsecond precision,
  `YYYYMMDDTHHMMSS.ffffffZ.md`. CLI creation holds the store lock and uses exclusive
  file creation, adding a zero-padded numeric suffix on collision. Clock rollback
  never replaces an existing file; lexical order describes wall time, not a guaranteed
  monotonic sequence. Stable `doc-N` IDs remain separate. Historical files and imports
  retain their paths; no migration or invented historical creation dates.
  A TUI draft that loses an ID/path race fails with its unsaved content retained.
- doc-30: use a complete directory scan at most once a second, with the original
  recursive/shallow/current-file scope. No new ignore policy is introduced.
  New entries append in sorted discovery order without moving existing selections.
  Read failures retain the project; active runs and pending agent edits defer refresh.
  F6 explicitly requests refresh. Dirty changed/deleted files retain the existing
  reload/keep-local decision. Confirming deletion discards that local buffer.
  Run evidence on disk is retained; deleted in-memory scenario rows are removed.
- doc-27: elapsed time is the observed task interval, including tool continuations
  and approval waits; the final elapsed value remains visible. Context uses reported
  last-input tokens and the existing context-window resolution, never an invented ACP
  estimate. Unknown usage is labeled unknown. Seventy percent shows a warning.
  Runtime/context are in the input title; cumulative input/output counts remain at
  widths that can display all three.
- doc-29: remove only the focus-dependent input-border shortcut title. Existing
  Send/Focus footer actions and keyboard semantics remain.
- doc-31/doc-36: Requirements, Testpoints and Explore use navigation on the left,
  main content above supporting details on the right. Below 70 columns or 12 body
  rows, show only the focused pane. Focus navigation preserves access to all three.
  Narrow AI hides the sidebar; keyboard agent switching remains available.
  Web/Desktop use a separate GPUI renderer and are not changed by this TUI layout.
- doc-34/doc-35: top controls select global context; footer controls act on the
  current tab. Use the existing action dispatcher and exact visible hit rectangles.
  Narrow Requirements/Testpoints retain concise clickable actions. Registered
  clickable controls share underlined hover feedback without changing selection.

## Product decisions

### doc-25 — approved and implemented

The user approved hiding the built-in Default suffix and displaying `Agent: <name>`
for custom profiles. Registry provenance distinguishes the actual built-in fallback
from a custom profile named Default. `/agent-profile` remains available.
The paired Features now describe the approved behavior; tests were not executed.

### doc-26 — approved and implemented

The user approved one shared status row. Activity, approval, errors and task recovery
use the left side; Agent/settings and fitting option chips share the same row.
The task status receives up to half the row (maximum 48 columns). Narrow controls
shorten Settings to an ellipsis and truncate long Agent names before omitting option
chips. Applying/Loading retain their own animated status. Settings exposes overflow.
Runtime/context remain in the input title. Native status rendering stays unchanged.
The paired Features now describe approved behavior; tests were not executed.

### doc-28

Recommendation: opt-in `teshi <project> — <running>/<total> running`, with error count
when present. Do not expose prompt text, session titles or model credentials. Update
only when the aggregate changes, at most once per second; clamp to 80 display columns.
Use OSC for terminal titles, with a separately configurable tmux pane-title policy.
Shell restoration cannot reliably recover an unknown initial title; prefer leaving
shell ownership as the default and require an explicit opt-in/restoration policy.
No terminal-title code is enabled until this decision is approved.

## Requirement mapping

All Feature names below have a corresponding Chinese Feature under `features/zh-CN`.
Each file declares its requirement ID and authored-only status. None has new confirmed
bindings or execution results. Scenario names and failure/boundary cases are in the files.

| ID | English Feature (features/en-US/) | Implementation | Status / remaining gap |
|---|---|---|---|
| doc-23 | requirement_creation.feature | engine authoring/create.rs; tui cli/requirements.rs, cli/mod.rs | Implemented; CLI runner steps and runtime acceptance pending |
| doc-24 | requirement_timestamp_names.feature | engine authoring/create.rs; tui authoring_tab.rs | Implemented with safe TUI draft conflicts; no historical migration |
| doc-25 | chat_profile_title.feature | agent loader.rs/registry.rs; tui ui/chat.rs | Implemented; title acceptance pending |
| doc-26 | chat_acp_status_row.feature | tui ui/chat.rs and acp_controls.rs | Implemented; narrow/status acceptance pending |
| doc-27 | chat_runtime_context.feature | tui app.rs; ui/chat.rs | Implemented; timer and ACP usage acceptance pending |
| doc-28 | terminal_title_policy.feature | This decision record only | Pending; terminal title behavior unchanged |
| doc-29 | chat_focus_hint.feature | tui ui/chat.rs | Implemented; send/newline behavior not exercised |
| doc-30 | external_feature_sync.feature | core gherkin.rs; tui app.rs, keymap.rs | Implemented; large-project scan cost and in-flight integration require verification |
| doc-31 | compact_terminal_layout.feature | tui ui.rs | TUI implementation; other GPUI surfaces not redesigned |
| doc-32 | explore_step_changes.feature | tui app.rs, ui.rs | Individual added/deleted Steps and attached arguments implemented, including deleted Features; diff/navigation acceptance pending |
| doc-33 | explore_horizontal_navigation.feature | tui keymap.rs, app.rs, ui.rs | Implemented; scrollbar and keyboard acceptance pending |
| doc-34 | footer_actions.feature | tui ui.rs | Existing action bars retained; narrow Requirements/Testpoints made clickable; generic editor/MindMap also expose Save, Model and Quit |
| doc-35 | pointer_feedback.feature | tui ui.rs | Shared registered-target, tree-row and ACP-chip feedback implemented; complete hover acceptance pending |
| doc-36 | combined_detail_layout.feature | tui ui.rs | Implemented for the three named tabs; resize/focus acceptance pending |
| doc-20 | winapp_owned_instances.feature | resources/winapp_service.py; tui cli/mod.rs, cli/winapp.rs | Executor-owned handle lifecycle implemented; ownership intentionally lost on executor restart |
| doc-21 | winapp_visual_exploration.feature | resources/winapp_service.py; tui cli/mod.rs, cli/winapp.rs | Window observation, guarded input, bounded plans and checker extension implemented; app-specific thresholds/Steam acceptance remain unverified |
| doc-22 | winapp_capability_diagnostics.feature | resources/winapp_service.py; CLI help; skills/teshi and skills/winapp-regression | Snapshot diagnostics and agent routing documented; three-Agent runtime evidence pending |

## Validation boundary

Formatting and a library compile check were performed; this is not acceptance.
New store concurrency/conflict test code is authored but not executed.
No unit tests, integration tests, Feature runs, PTY sessions or real-window checks
were executed. No requirement was marked verified or accepted.

This round did not execute tests; all runtime outcomes await later verification.
