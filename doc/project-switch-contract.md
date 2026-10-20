# TUI project switching contract

## Ownership and activation

`App::request_project_switch` resolves the target identity and collects dirty
objects. `ProjectSwitchState` owns the pending request and the active identity
and generation. `prepare_project_context` builds B without replacing A;
`commit_project_switch` installs B only after preparation and task shutdown.
The global process working directory is not changed.

Detached preparation uses `teshi_core::gherkin::try_parse_project`: directory
traversal, Feature reads and UTF-8 decoding must succeed before installation.
Errors include the affected path and keep A active; no partial project is
returned. Incomplete Gherkin remains editable. Canonical visited directories
prevent alias cycles. Recent-history persistence happens after activation, so
a history write failure warns without undoing the successful switch.

Requirement drafts and test-point fields remain dirty after persistence fails.
Cancelling a switch retains their editor state; restoring the destination and
retrying saves to the shared requirement store or A's point file respectively.
Meaning-bearing point edits remain Proposed and do not approve human review.

| State in `App` | Owner and initialization | Persistence / cancellation | Activation rule |
| --- | --- | --- | --- |
| `project`, `git_status`, `step_index`, `mindmap_index` | Project; target Feature sources | Git refresh is a read-only worker with a captured root and stamp | Replace indexes; reject old refresh stamps |
| `git_status_rx`, revision, refresh flags/deadline | Project; current Git snapshot | Detached reads may finish; they cannot mutate B | Drop old receiver; reset scheduling/revision |
| `buffers`, `buffer`, `buffer_dirty`, `dirty`, `disk_stamps`, `file_path`, `active_buffer_idx` | Project; target Feature files | Dirty objects resolved by save/discard/cancel | Replace only after B prepares; failed preparation preserves A |
| `undo_stack`, `redo_stack`, step input/picker, pending key, folds, source selection, scenario focus/dropdown | Project view; editor actions on A's source | No disk persistence; keep on cancel/preparation failure | Clear at successful activation; undo cannot restore A in B |
| Cursor, scroll, tree state, preview, diagnostics and registered source hit regions | Project view; parsed source/render | No persistent writes | Reset/rebuild against B |
| `runner_config`, `runner_rx`, `runner_cancel`, Explore run maps/results/details | Project; explicit root/config and owned runner | Stop sender targets owned work; wait for `RunnerExit` and output readers | No receiver/config/result carryover |
| Explore selection memories, edit mode, horizontal offsets/drag, viewports and pending change summary | Project view; source indices and render | No disk persistence | Clear/reset against B |
| `agents`, pending mutations, pending diffs, visual observations, permissions and history restoration | Project/session; target root and matching saved session | Stop transports/workers and MCP bridge; proposals still require explicit review | Fresh backend/context; no old permission or proposed mutation accepted |
| `generation_stage`, pipeline requirement/plan, source scope/prompt and pause reason | Project task; confirmed generation sources | Shutdown preserves stored Proposed test points; no approval synthesized | Reset; restore only B's persisted generation state |
| `authoring_ui`, `test_points_ui` | Mixed: global requirement root, project-local points/links and editor drafts | Unsaved drafts/point edits participate in dirty resolution | Reload from B plus the same global requirement root |
| Session browser/list, selected history, model caption | Project view; normalized session ownership | Checkpoint A before preparation; legacy owner remains absent | Reload B list; clear restoration and caption |
| `agent_registry` | Project configuration plus shared installed-agent definitions | Load using explicit target root | Replace resolved registry; preserve installed-agent registry |
| `project_switch` | Global coordinator plus active project stamp | Recent history written only after activation | Advance generation once; same identity is a no-op |
| Theme/config, model profiles/forms, credentials, installed ACP agents, requirement store root | Global | Existing global persistence; no project teardown | Preserve |
| In-process/system clipboard ownership, top-level tab and general UI preferences | Global/view | Clipboard ownership retained for X11 | Preserve; source selections themselves are cleared |

Source references: `crates/teshi-tui/src/app.rs`, `project_context.rs`,
`crates/teshi-engine/src/project_identity.rs`, and `app_data.rs`.

## Task shutdown boundary

External runners use their owned PID/tree on Windows and a dedicated process
group on Unix. `RunnerExit` follows process wait and output-reader completion.
ACP uses a non-breakaway Windows Job assigned before the suspended primary
resumes; shutdown waits for an empty Job, including tools left after primary
exit. Unix ACP uses a dedicated process group.

Unix support covers ordinary descendants that remain in the owned group.
Deliberate `setsid`/`setpgid` escape and externally delegated work are unsupported,
as explicitly accepted by the user on 2026-10-20. This is not an arbitrary-agent
sandbox. Process-name scanning, global subreaping and killing unrelated daemons
are not substitutes for owned process containment.

Native workers retain their handles while cancellation interrupts stalled reads.
ACP rejects old queued calls and events while shutdown is pending. The switch
wait is bounded: failure/timeout retains A and owned handles. Synchronous
dispatch can finish against A before retrying activation; a timed-out switch
must not relabel that dispatch as B's work. Permission rejection and Proposed
test-point review remain explicit human actions.

## Evidence and limits

The handoff's authoritative scoped inventory and dated acceptance reports identify
the exact tested sources/binaries. Unit tests, mechanism probes, focused product
PTYs and complete Feature replays are separate forms of evidence. Neither an
unchecked historical checkbox nor one successful focused replay certifies the
entire dirty checkout. See `docs/requirement-implementation-handoff-2026-10-16.md`
and `docs/project-switch-evidence-reconciliation-2026-10-20.md`.
