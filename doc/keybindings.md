# TUI keybindings

Bindings are resolved from the active modal, pane, and input mode. Uppercase
letters mean Shift+letter. `dd`, `gg`, and `Ctrl+W h` are sequential keys.

## Modes and common navigation

Normal/Browse mode owns commands. Insert/Input mode owns printable characters,
including `hjkl`, `q`, `s`, and digits. `Esc` returns to Normal or closes the
current modal layer. Requirements calls its Normal mode **Browse**.

| Key | Action |
|-----|--------|
| `1` / `2` / `3` / `4` / `5` | Explore / MindMap / AI / Requirements / Test Points, in Normal mode |
| `Ctrl+W`, then `h/j/k/l` or Left/Down/Up/Right | Focus the adjacent visible pane; stop at edges |
| `Esc` after `Ctrl+W` | Cancel the prefix |
| `j/k` or Down/Up | Navigate lists and trees |
| `hjkl` or arrows | Move the cursor in a Normal-mode text editor |
| `gg` / `G` | First / last item or document position, except the Requirements tree reserves `g` for grouping |
| `Home/End` | First/last list item; line boundaries inside text editors |
| `Ctrl+B/Ctrl+F` or PageUp/PageDown | Page up/down in Normal mode and choice lists |
| `s` / `Ctrl+S` | Save the active editable buffer in Normal mode |
| `m` | Open model settings in Normal mode |
| `q` / `Ctrl+C` | Request exit; modal panels may capture input first |

Pane chords do not steal text input or Requirements visual selections. Existing
Tab/Shift+Tab and non-text column-navigation keys remain available. Layouts with
no vertical neighbor ignore `Ctrl+W j/k` and `Ctrl+W Up/Down`.

All five tab labels and the project header are clickable. Narrow terminals use
Explore, Map, AI, Req, and Points labels. Wheels target the pane beneath the
pointer; Shift+wheel scrolls horizontally where supported.

## Explore

### Browse

| Key | Action |
|-----|--------|
| `j/k`, Down/Up | Select next/previous row |
| `h/Left`, Shift+Tab | Previous column |
| `l/Right` | Navigate right according to the selected column |
| Tab | Next column |
| Home/End, `gg/G` | First/last row |
| PageUp/PageDown, Ctrl+B/Ctrl+F | Navigate approximately ten rows |
| Enter | Expand/collapse failure details |
| `r` | Run the selected scenario |
| `p` | Toggle plaintext for redacted HTTP inspector fields |
| `a` | Request an AI suggestion |
| `e` | Open the selected file editor |
| Esc | Clear the current interaction state |

### Feature editor: Normal

| Key | Action |
|-----|--------|
| `hjkl`, arrows | Move the selected line or keyword/body focus |
| Home/End | First/last navigable position |
| `gg/G` | First/last navigable position |
| PageUp/PageDown, Ctrl+B/Ctrl+F | Page through navigable lines |
| `i` / Enter | Edit the selected step or title; a keyword focus opens its picker |
| Space | Fold/unfold the current scenario |
| Ctrl+Space | Fold/unfold all scenarios |
| `o/O` | Insert a step below/above |
| `dd` | Delete the current step or scenario |
| `yy` / `p` | Copy/paste a step |
| Ctrl+Up/Down, Ctrl+K/J | Move the step block up/down |
| Shift+Up/Down, `K/J` | Previous/next sibling step |
| Ctrl+N | New scenario |
| Ctrl+G / Ctrl+T / Ctrl+A | Set Given / Then / And |
| F5 | Run in the background |
| `u` / Ctrl+R | Undo/redo |
| Ctrl+/ / Ctrl+Y | Compatibility undo/redo |
| `s` / Ctrl+S | Save |
| Esc | Leave the current editor state |

`Ctrl+W` is exclusively a pane prefix; the old When shortcut is removed. Use
the keyword picker to select When. The default editor branch retains the legacy
Ctrl+backslash control-character binding for the MindMap AI panel.

### Feature editor: Insert

Printable characters insert text. Arrows and Home/End move the cursor.
Enter commits the line; Tab splits or inserts a step line. Backspace/Delete
remove characters or merge lines at a boundary. Ctrl+S saves; Esc leaves input.
Ctrl+/ undoes and Ctrl+Y redoes. Undo also accepts legacy terminal encodings
Ctrl+7, Ctrl+underscore, and Ctrl+Shift+minus.

## MindMap

| Key | Action |
|-----|--------|
| `j/k`, Down/Up | Next/previous node |
| `h/l`, Left/Right | Collapse/expand |
| Space | Toggle expansion |
| `J/K`, Shift+Down/Up | Next/previous sibling |
| `gg/G`, Home/End | First/last node |
| PageUp/PageDown, Ctrl+B/Ctrl+F | Page through nodes |
| `[` / `]` | Previous/next associated location |
| `a` | Send the selected node to AI |
| `p` | Show/hide the AI panel |
| Enter | Focus the visible AI panel |
| `o` | Open the scenario-location dropdown |

The tree is display-only. Its AI panel uses the chat Normal/Input rules below.
Esc from the panel's Normal mode returns focus to the tree; Esc from its input
first leaves Input. In panel Normal mode, `o` also opens the location dropdown.

## AI chat (AI tab and MindMap AI panel)

### Normal

| Key | Action |
|-----|--------|
| `j/k`, Down/Up | Scroll vertically |
| `h/l`, Left/Right | Scroll horizontally |
| Alt+arrows | Scroll in the corresponding direction |
| `gg/G`, Home/End | History top/bottom |
| PageUp/PageDown, Ctrl+B/Ctrl+F | Scroll history |
| `i` / Enter | Focus input |
| Ctrl+Shift+C | Copy the chat selection |
| Esc (AI tab, request running) | Cancel the current request |

Only on the AI tab: `[`/`]` select the previous/next Agent conversation,
`a` creates one, and `x` closes the current one. `j/k` no longer switch Agents.

### Input

| Key | Action |
|-----|--------|
| Printable characters | Insert text |
| Enter | Send; empty input returns to Normal |
| Ctrl+J / Shift+Enter | Insert a newline |
| Left/Right, Up/Down | Move input cursor; Up/Down move between input lines |
| Home/End | Move to the start/end of the current input line |
| Backspace/Delete | Delete text |
| Ctrl+V | Paste clipboard text |
| Alt+arrows | Scroll history |
| PageUp/PageDown, Ctrl+K | Scroll history vertically |
| Ctrl+Home/End | History top/bottom |
| Ctrl+Shift+C | Copy chat selection |
| Esc | Return to Normal |

In the AI tab, Esc from Input followed by Esc in Normal cancels a running
request. Ctrl+C requests exit instead. Terminal support is needed to distinguish
Shift+Enter from Enter; Ctrl+J also works in terminals without that support.

Drag over chat text to select it; Ctrl+Shift+C or right-click copies the displayed
plain text without role rails or soft-wrap line breaks. Scrolling/resizing keep
valid selections; changed streamed text or a different session clears them.

## Requirements

### Tree and linked test points

| Key | Action |
|-----|--------|
| `j/k`, Down/Up | Next/previous item |
| Left/Right | Previous/next pane (compatibility) |
| Ctrl+W then `h/l` or Left/Right | Previous/next pane |
| Home/End | First/last item |
| PageUp/PageDown, Ctrl+B/Ctrl+F | Page through items |
| Ctrl+N | New requirement Markdown document |
| `d` / Delete | Open global-document deletion confirmation |
| `i` | Iteration filter |
| `o` | Cycle Current project / All projects / No project |
| `b` | Link/unlink the saved document to/from the current project |
| `g` | Toggle path vs iteration/path grouping |
| `I` | Edit iteration |
| `p` | Edit optional priority (Unset, P0-P3) |
| `s` / Ctrl+S | Save |

Bare `h/l` do not change panes here. The tree reserves `g` for grouping, so use
Home/End for its boundaries. Priority changes preserve document order, Markdown,
and unsaved text; existing documents remain unset.

### Markdown Browse

| Key | Action |
|-----|--------|
| `hjkl`, arrows | Move cursor |
| `w/b/e` | Next word / previous word / word end |
| `0` / Home, `^`, `$` / End | Line start / first nonblank / line end |
| `gg/G` | Document top/bottom |
| PageUp/PageDown, Ctrl+B/Ctrl+F | Page through lines |
| `i/a` | Insert at/after cursor |
| `o/O` | Open a line below/above and enter Insert |
| `x`, `dd` | Delete character / line |
| `yy`, `p/P` | Yank line / paste after/before |
| `u`, Ctrl+R | Undo/redo |
| `v/V` | Character/line selection |
| `n` | Create a Proposed test point from a nonempty selection |
| Ctrl+N | New requirement document |
| `I` | Edit iteration |
| `s` / Ctrl+S | Save |
| Esc | Return to Browse / clear visual state |

Visual mode supports arrows/hjkl, w/b/e, 0/^/$ and End to extend the selection,
`y` to yank, `d` to delete, and Esc to return to Browse. This is a bounded Vim
subset: no counts, macros, named registers, search/replace, Ex commands, or full
operator-motion grammar.

### Markdown Insert

All printable ASCII and Unicode is text. Arrows and Home/End move the cursor;
Enter inserts a newline, Tab inserts four spaces, Backspace/Delete edit, and
Ctrl+S saves without leaving Insert. Esc returns to Browse. Bracketed paste is
accepted. Long lines soft-wrap; the editor and adjacent panes expose scrollbars.

## Test Points

### Normal

| Key | Action |
|-----|--------|
| Tab/Shift+Tab | Next/previous pane |
| Ctrl+W then `h/l` or Left/Right | Previous/next pane |
| `j/k`, Down/Up | Select a point, a detail field, or an excerpt |
| `h/l`, Left/Right | Move the field cursor in Details; otherwise change panes |
| `gg/G` | First/last point, field, or excerpt |
| Home/End | Field line boundaries in Details; first/last list item elsewhere |
| PageUp/PageDown, Ctrl+B/Ctrl+F | Page through points, fields, or excerpts |
| `i` / Enter (Details) | Enter Insert for the selected field |
| `a/A` | Approve selected / batch-approve visible eligible points |
| `r` | Reject the selected point |
| `f` | Cycle review-state filter |
| `c` | Continue generation after human approval |
| `o` | Open the selected realized Gherkin scenario |
| Enter (Excerpts) | Follow the excerpt into Requirements |
| `s` / Ctrl+S | Save |

### Insert (Details)

All printable characters, including commands and tab digits, insert text.
Left/Right move by Unicode characters; Up/Down move within multiline text;
Home/End move to logical line boundaries. Backspace/Delete edit at the cursor.
Shift+Enter inserts a newline. Esc or Enter commits the field and returns to
Normal. Ctrl+S saves while staying in Insert; a save that changes the active
filtered selection can return to Normal. Tab/Shift+Tab commits and moves
pane focus. Bracketed paste inserts text without executing review commands.

Editing meaning-bearing fields still resets locked review states to Proposed;
keyboard navigation never approves points automatically.

## Dialogs and pickers

Choice lists support `j/k` and Up/Down, `gg/G` and Home/End, and
Ctrl+B/Ctrl+F or PageUp/PageDown unless noted otherwise. Enter selects; Esc
cancels/closes. Text forms preserve printable `j/k`.

| Dialog | Additional keys / behavior |
|--------|----------------------------|
| Project picker | Up/Down or j/k, Enter to open, `o/O` to type a path; Esc cancels |
| Project path | Printable text, including j/k; Backspace deletes; Enter switches; Esc cancels |
| Dirty project switch | S save-and-switch, D discard-and-switch, Esc cancel |
| Busy project switch | S stop-and-switch, Esc cancel; timeout leaves current project active |
| External file changed | Enter/r reload; Esc/k/K keep local |
| Agent change review | Y accept, N reject, D diff, S change summary; Esc blurs input first, otherwise rejects |
| Change Summary | List navigation; Enter jumps to the MindMap node |
| ACP permission | List navigation; Enter submits the highlighted option; Esc rejects |
| Quit confirmation | h/l or Left/Right select a button; Enter/q executes it; y/Y always exits; n/N/Esc cancels |
| Model/session deletion | Requested with dd, Delete, or the button; Enter/y confirms; Esc/n cancels; background input blocked |
| Step keyword picker | List navigation; Enter applies; Esc cancels |
| Scenario location | List navigation; Enter selects; o or Esc closes |
| Slash suggestions | Up/Down selects; Enter/Tab executes; Esc dismisses; other keys continue typing |
| Requirement iteration filter | List navigation; Enter applies |
| Requirement iteration edit | Text and Backspace; Enter saves (empty = Unassigned); Esc cancels |
| Requirement priority | List navigation over Unset/P0-P3; Enter applies |
| Requirement deletion | Enter deletes; Esc cancels |
| Unsaved requirement | S saves and continues, D discards and continues, Esc cancels; save failure keeps the prompt |
| Generation scope | Enter starts, i changes iteration, Esc cancels |
| Model list | List navigation; Enter activates, a adds, e edits, dd/Delete requests deletion |
| Model form | Tab/Shift+Tab or Down/Up switch fields; Left/Right/Home/End move the cursor; Backspace/Delete edit; Enter or Ctrl+S submits; Esc cancels |
| Session history | List navigation; Enter resumes, dd/Delete requests deletion |
| Approval mode | List navigation; Enter applies Manual/Auto/Bypass |
| Agent selection | List navigation; Enter uses Native or the installed ACP agent |
| Installed ACP agents | List navigation; Enter selects; a/i opens the registry |
| ACP registry | List navigation; Enter installs the selected agent |
| Native Agent Profiles | List navigation; Enter selects; no active add/edit/delete form |
| ACP settings | List navigation; h/Left/Backspace returns; l/Right/Enter opens/applies; a opens Agent selection; p opens approvals |
| Authentication panel | Esc closes |

Ctrl+comma or `/settings` opens ACP settings. Other active modals keep ownership
of their input. Terminals that cannot encode Ctrl+comma can use `/settings`.
Agent mode settings and Teshi approval modes remain separate controls.

## AI slash commands

| Command | Action |
|---------|--------|
| `/new` | New session |
| `/exit` | Exit Teshi |
| `/resume` | Resume the current project's most recent session |
| `/retry` | Continue an interrupted ACP task in the current session |
| `/copy`, `/copy N` | Copy the latest / last N completed nonempty assistant answers |
| `/details` | Toggle full thinking and tool details |
| `/models` | Native model profiles or ACP model settings |
| `/sessions`, `/sessions legacy` | Project conversations / explicitly unassigned legacy history |
| `/project`, `/project <directory>` | Project picker / direct project switch |
| `/approval` | Approval modes |
| `/agent` | Session agent backend |
| `/acp` | Installed ACP agents and registry |
| `/settings` | ACP model, mode, thinking, and other advertised settings |
| `/agent-profile` | Native Agent Profile selection |
| `/generate` | Start requirements → test points → scenarios generation |
| `/continue` | Continue generation after the human test-point review gate |

Conversations checkpoint every second and before normal lifecycle transitions.
Restoring ACP history retains agent selection and supplies saved text to a new
process; it does not restore its prior tool state or transient settings. Compatible
reasoning and tool progress are displayed in chat. `/copy` excludes partial text,
reasoning, and tools. Interrupted ACP tasks also expose a clickable Continue task
button; continuing is explicit and does not bypass generation review.
