# Keybindings

## Tab switching

| Key | Tab |
|-----|-----|
| `1` | Explore |
| `2` | MindMap |
| `3` | AI |
| `4` | Requirements |
| `5` | Test Points |

All five tab labels are clickable. Terminals narrower than 85 columns use
`Explore`, `Map`, `AI`, `Req`, and `Points` labels with the same shortcuts.

`Ctrl+W` then `h`/`l` moves between visible panes on the active tab and stops
at the edge. `Esc` cancels the prefix. `j`/`k` after the prefix is a no-op
when the layout has no vertical neighbor. Text entry and modal panels keep
their own input behavior. A wheel targets the pane under the pointer;
Shift+wheel scrolls horizontally where supported.

## Project switching

The project header is clickable on every tab. Click it or enter `/project` in
the AI input to open the recent-project picker. `↑`/`↓` or `j`/`k` select a
recent entry, `Enter` opens it, and `Esc` cancels. `[O] Open path` accepts a
directory typed into the path field; printable path characters, including
`j`/`k`, are inserted there rather than treated as picker navigation.

`/project <directory>` requests a direct switch. A dirty project shows `S`
save-and-switch, `D` discard-and-switch, and `Esc` cancel. Running project-owned
tasks show `S` stop-and-switch and a bounded wait; a timeout leaves the current
project active. A missing or unreadable target reports an error and keeps the
current project open.

## Navigation (Explore / MindMap)

| Key | Action |
|-----|--------|
| `↑` / `↓` / `j` / `k` | Previous / next navigable line or tree node |
| `←` / `→` / `h` / `l` | Toggle keyword vs body focus; move between columns |
| `Tab` / `→` | Move to next column |
| `BackTab` / `←` / `h` | Move to previous column |
| `Home` / `End` | First / last node or line |
| `PageUp` / `PageDown` | Scroll ~10 nodes or lines |

## Requirements tab

The footer provides clickable New, Delete, Filter, Group, Iteration, Save,
and Point actions with their keyboard shortcuts. Dirty-buffer and delete
confirmations provide clickable choices; their background panes stay blocked.
Linked test-point rows can be selected with the pointer before navigation.

The Markdown editor starts in **Browse** mode. Press `i` in the editor to enter
**Insert**, and `Esc` to return to Browse. Browse never inserts ordinary text.

Use `Ctrl+W` followed by `h`/`l` to move focus to the previous/next pane;
bare `h`/`l` do not switch panes.

| Key | Action |
|-----|--------|
| `←` / `→` | Move between panes from the tree or linked test points |
| `hjkl` / arrows | Move the cursor in Browse |
| `n` | Create a Proposed test point from the active non-empty selection |
| `Ctrl+n` | Create a new requirement Markdown document |
| `i` | Filter by iteration in the tree; enter Insert in the editor |
| `g` | Toggle grouping: path hierarchy vs iteration → path |
| `I` | Edit iteration from the tree or Browse editor (empty = Unassigned) |
| `s` | Save the current requirement document in Browse |

In Insert, all printable ASCII and Unicode characters are text, including
`s`, `q`, `hjkl`, and `1`–`5`. Arrows, Home/End, Enter, Backspace, and Delete edit
the document; `Tab` inserts four spaces. `Ctrl+S` saves and stays in Insert.
Bracketed paste accepts text in Insert.
Mouse dragging selects Markdown text for creating a test point in Browse.
The Markdown editor soft-wraps long lines and shows a vertical scrollbar when
content overflows. The tree and linked test points support both scrollbar axes. Click or drag a scrollbar to scroll that pane. The mouse
wheel scrolls vertically; `Shift` + wheel (or a horizontal wheel) scrolls
horizontally. Keyboard navigation keeps the selected row or editor cursor visible.

Leaving a dirty document through document selection, a filter that hides it,
main-tab navigation, new-document creation, or quit opens an unsaved-changes
prompt: `S` saves and continues, `D` discards and continues, and `Esc` cancels.
A failed save keeps the buffer and prompt open so the operation can be retried.

### Requirements Vim subset

Requirements Browse mode supports `i`, `a`, `o`, and `O` for entering or opening
lines; `Esc` returns to Browse. Movement includes `h/j/k/l`, `w/b/e`, `0`, `^`,
`$`, `gg`, and `G`. `x`, `dd`, `yy`, `p`, `P`, `u`, and `Ctrl+R` provide local
editing, yank, paste, and undo/redo. `v` and `V` select characters or lines;
`y` and `d` operate on the visual selection. `Ctrl+S` saves.

Counts, macros, named registers, search/replace, Ex commands, and full Vim
operator-motion grammar are outside this subset. Uppercase `I` remains the
existing iteration editor.

## Test Points tab

| Key | Action |
|-----|--------|
| `Tab` | Cycle tree / details / excerpts panes |
| `a` | Approve the selected test point (resolved links only) |
| `A` | Batch-approve visible eligible test points |
| `r` | Reject the selected test point |
| `c` | Continue generation after approving test points (human-only gate) |
| `o` | Open a realized Gherkin scenario linked from the selected test point |
| `f` | Cycle review-state filter |
| `s` | Save test-point edits |
| `Enter` | Follow the selected requirement excerpt into Requirements |

## Editing (editor / step body mode)

| Key | Action |
|-----|--------|
| `e` | Enter editor for selected file |
| `Enter` | Open step edit or commit active line edit |
| `Space` | On keyword: open step keyword picker; on body: start editing |
| `Tab` | Insert new step line (splits or inserts below) |
| `Backspace` / `Delete` | Delete character or merge lines |
| `Esc` | Clear input state / close overlays |
| `d` `d` | Delete current step or scenario |
| `y` `y` | Copy current step |
| `p` | Paste copied step |

## ACP session settings

Select an external agent with `/agent`. Click the bottom model, mode, thinking,
or other advertised setting to choose a value. `More` opens the full list when
the terminal is narrow. `/settings` and `Ctrl+,` also open the list; terminals
that do not encode `Ctrl+,` can use `/settings`. `/models` and the existing
model shortcut open the external agent's model choices in ACP conversations.

In a settings picker, use Up/Down (or j/k), Enter to apply, Left/Backspace to
return, and Escape to close. `a` opens agent selection and `p` opens Teshi
approvals. Agent modes and Teshi approvals are separate controls. Busy turns
and pending settings changes prevent conflicting mutations.

## AI conversation history

The AI chat streams `reasoning_content` supplied by compatible native providers
and thoughts supplied by ACP agents. It also shows native tool
requests and results, and retained ACP tool progress. Tool updates with the same
ACP call ID update their existing entry, including updates that omit the title.
Consecutive ACP tools share a progress group. Compact rows keep their status
visible and shorten long titles to the available terminal width. `/details`
toggles full thinking text and ACP tool titles for the current conversation.

Conversations are checkpointed every second while the TUI runs, including partial
replies, and saved before starting a new conversation, closing an agent, loading
another session, or exiting normally. Each conversation keeps one stable file in
the user configuration directory under `teshi/sessions`. `/sessions` lists the
current project's conversations; `/sessions legacy` explicitly lists older
unassigned conversations. `/resume` loads the most recently updated conversation
for the current project. Existing session files remain readable.

Drag over chat text in AI or the MindMap AI panel to select it. `Ctrl+Shift+C`
or a right click copies the selection. Copy uses the displayed plain text,
including visible code indentation and language labels, without role rails or
soft-wrap line breaks. Scrolling and resizing preserve valid selections;
stream changes to the selected content and session changes clear them.

`/copy` copies the latest completed nonempty assistant answer; `/copy N` copies
the last N answers in chronological order. Live partial answers, reasoning and
tool activity are excluded. Invalid counts and an empty answer history produce
feedback. A clipboard write failure keeps the selection available for retry.

Restoring an ACP conversation retains its installed agent selection. A new ACP
process receives the saved text history on its first prompt; this does not restore
the previous external process, its tool state, or transient session settings.

## Structural editing

| Key | Action |
|-----|--------|
| `Ctrl+/` | Undo (full buffer snapshot) |
| `Ctrl+Y` | Redo |
| `s` | Save current file |
| `q` | Quit (press twice if buffer is dirty) |

### Interrupted ACP tasks

When an ACP agent ends with `refusal`, `max_tokens`, or `max_turn_requests`,
chat retains an interruption notice and shows a **Continue task** button.
Click it or submit `/retry` to continue in the current session, asking the agent
to verify completed operations before proceeding. This is an explicit new turn;
Teshi does not automatically replay tools or infer authentication failure from
`refusal`. `/continue` retains its test-point generation review behavior.
