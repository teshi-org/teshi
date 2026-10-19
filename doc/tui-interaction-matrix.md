# TUI interaction matrix

This matrix records the interaction contract implemented by the terminal UI. Pane names refer to the rectangles produced during the current render pass; hit testing must use those rectangles rather than a remembered layout from an earlier tab or terminal size.

## Main tabs

| Tab | Visible panes and keyboard order | Selectable controls | Mouse regions and wheel target | Text selection |
| --- | --- | --- | --- | --- |
| Explore | Feature → Scenario → Step; editor mode exposes the editor and reserved panel | Feature, deleted feature, scenario, step, run/detail controls, horizontal bars | Registered row rectangles; a wheel over a column scrolls that column; a bar drag owns its column | Editor source selection in edit mode |
| MindMap | Tree → right side when the AI panel is visible; the right side renders preview above chat | Tree nodes, location dropdown, AI controls, chat text | Tree rectangle, ACP/chat controls, chat drag selection; wheel over tree, preview, or chat acts on that rectangle | Chat source selection in the chat rectangle |
| AI | One chat/history pane and input | Agent/session/model/approval controls, ACP controls, chat text | Controls register their exact rows; wheel over history scrolls history; chat drag selects | Chat source selection |
| Requirements | Tree → Markdown editor → linked test points; narrow terminals collapse the last two into editor | Tree documents, filter/group/iteration/delete/new/test-point controls, editor text, linked rows | `pane_areas` are refreshed every render; wheel and scrollbar drag use the pane under the pointer | Markdown selection in the editor; editor display rows map back to source characters |
| TestPoints | Tree → Intent & review → Requirement excerpts; narrow terminals collapse details and excerpts | Tree test points, detail fields, review/filter actions, scenario/excerpt rows | Pane rectangles and registered row controls; wheel is scoped to the pane under the pointer | Detail text uses its field buffer; excerpt text is navigational |

Labels and explanatory status/footer text are intentionally noninteractive unless listed in the table. Modal panels and ACP pickers replace this matrix temporarily: the modal owns keyboard and pointer input, and the underlying pane is not dispatched.

## Shared keyboard contract

`Ctrl+W` starts a two-key pane prefix in browse/navigation states. `h`/`l` move to the previous/next visible pane and clamp at the edge. `j`/`k` target the nearest pane below/above; the current layouts have no unambiguous vertical focus neighbor, so those actions are consumed as documented no-ops. `Esc` cancels a pending prefix without editing or switching tabs. Windows uppercase `Ctrl+W` events are normalized.

Text input has precedence over the prefix: step editing, Requirements insert mode, TestPoints detail editing, and focused AI input retain their input semantics. Modal overlays, ACP permission prompts, profile/session pickers, and the requirements filter/iteration overlays intercept the same keys before pane navigation.

The action is shared by the keyboard and pointer paths: a pointer click changes the same selection/focus state as the corresponding list control. Scrollbars are rendered only for overflow. Their track and thumb belong to the owning pane; dragging one cannot start text selection or activate a neighboring control. Shift+wheel changes the horizontal offset where the pane supports horizontal scrolling.

## Layout and minimum-size checks

The Requirements tree and linked panes keep independent horizontal offsets. The Markdown editor uses a source-aware soft-wrap map and therefore exposes vertical overflow without an editor horizontal bar. Explore columns expose horizontal bars only when their content exceeds the rendered column. Scrollbar geometry clamps zero-length tracks, one-cell tracks, and full-width thumbs; CJK and wide characters are measured in terminal cells while selections remain character-based.

Acceptance captures should be made after rendering at both the normal fixture size and the smallest supported terminal size. A direct action or handler unit test is useful for state semantics, but pointer acceptance must also use a real terminal or the runner's mouse adapter.

## Verified scoped delivery — 2026-10-19

The isolated committed baseline plus the P4 changes passes 75 real Windows PTY checks: AI controls 14, management forms 14, Explore 3, navigation/pickers 9, shared overlays 8, Requirements controls 15, TestPoints controls 5, shared tab/pane interaction 4 and requirement creation/scrolling 3. Each suite saves real viewport and persisted-data evidence under `.teshi/artifacts/p4-scoped-20261019/pty/`.

The same pinned CLI also passes all eight chat-selection and eight Requirements Vim regressions, including exact desktop clipboard readbacks, Unicode and resize behavior, local undo and saved Markdown. TUI unit tests pass 442 / one ignored; affected-package formatting, TUI all-target Clippy and the nine pointer-suite Clippy checks pass.

The registry picker uses an owned local HTTP fixture through `TESHI_ACP_REGISTRY_URL`; without this setting the official registry remains the default. The Explore pointer fixture starts in its project and verifies the selected runner case and process cwd. Starting outside a project and switching project context are separate P8 contracts. Session modal checks use the visible Load action rather than a P8-specific title.

P6 priority and the P8 header/picker, lifecycle and session-ownership changes are excluded from this delivery. These checks establish functional Windows behavior at the tested sizes. User aesthetic acceptance, other platforms and the complete Feature/provider matrix remain separate acceptance work. Initial extraction and fixture failures are retained alongside the passing candidate evidence; current completion must use the candidate results, not those initial runs.
