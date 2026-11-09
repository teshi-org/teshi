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
| `Ctrl+Q` or `:q` | Exit; the `^q 退出` control at the bottom-right of every tab is the clickable equivalent. `:q` is the host-safe route when a parent terminal also binds `Ctrl+Q`. `q` and `Ctrl+C` never exit, and Ctrl+C copies the selection |

Pane chords do not steal text input or Requirements visual selections. Existing
Tab/Shift+Tab and non-text column-navigation keys remain available. Layouts with
no vertical neighbor ignore `Ctrl+W j/k` and `Ctrl+W Up/Down`.

All five tab labels and the project header are clickable. Narrow terminals use
Explore, Map, AI, Req, and Points labels. Wheels target the pane beneath the
pointer; Shift+wheel scrolls horizontally where supported.

### Command line

`:` in any Normal/Browse mode opens a single-line prompt in the footer strip.
Insert, Input, and form modes keep `:` as a literal character.

| Key | Action |
|-----|--------|
| `:` | Open the prompt in Normal/Browse mode |
| `Enter` | Run the command and close the prompt |
| `Esc` | Close the prompt without running it |
| `Backspace` | Delete the character before the cursor; on an empty prompt it closes the prompt |

| Command | Action |
|---------|--------|
| `:q`, `:quit` | Quit through the same unsaved-changes guard as `Ctrl+Q` |
| `:w`, `:write` | Save the active editable buffer, like `Ctrl+S` |
| `:wq`, `:x` | Save, then quit; a failed save keeps Teshi open |
| anything else | Report the unknown command in the footer and stay open |

The prompt is the reliable exit inside a host terminal that swallows `Ctrl+Q`,
because it needs no control chord. `Ctrl+Q` and the footer exit control still
work while the prompt is open, and a left click closes it.

## Explore

### Browse

| Key | Action |
|-----|--------|
| `/` | Search Feature names, titles, and relative paths in the Features column; Enter keeps the filter, Esc clears it |
| Enter, Left/Right | Toggle, collapse, or expand the selected Feature directory |
| `j/k`, Down/Up | Select next/previous row |
| `h/Left`, Shift+Tab | Previous column |
| `l/Right` | Navigate right according to the selected column |
| Tab | Next column |
| Home/End, `gg/G` | First/last row |
| PageUp/PageDown, Ctrl+B/Ctrl+F | Navigate approximately ten rows |
| Enter | Expand/collapse failure details |
| `i` | Open the replay screenshot viewer for the selected step, or for the failed step while Failure Details is open |
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
| `i` | Toggle the replay screenshot viewer |

The viewer also opens from Explore, which is the main entry point: the
Scenarios and Steps panes mark captures with `●N`, `i` on the Steps pane opens
the capture of the selected step, and `i` in Failure Details opens the failing
step. In Explore the viewer covers the Scenarios and Steps panes, leaving the
Features column as context; narrow terminals fall back to the whole tab.
Closing the viewer moves the Steps selection to the capture that was last
shown. The MindMap entry stays available for browsing evidence by requirement
structure.

The viewer shows the captures recorded for the selected scenario by
the newest replay run that has them. `j/k` switch between steps. `+`/`=` and
`-` zoom, `1` selects 100 percent, and `0`/`f` resets to fit. In a zoomed view,
arrow keys pan through original pixels; `h/l` also pan horizontally. In fit
view, Up/Down retain step navigation. `o` opens the original in the system
viewer, and `i` or Esc closes it, including while an image is loading.
Loading and encoding run in the background; the title distinguishes loading
from incremental pixel output. See [screenshot viewing](tui-image-viewer.md)
for limits and validation.
Terminals without an image protocol (kitty, sixel or iTerm2) show the capture
path instead of the picture; `o` still works there.

Windows Terminal 1.22+ supports Sixel. On Windows, Teshi queries device
attributes through the console before starting its input reader. Sixel does
not require a font-size reply; Windows Terminal uses a 10 by 20 virtual pixel
grid per cell. `WT_SESSION` alone does not establish graphics support.
`NO_COLOR`, `TERM=dumb`, `TESHI_NO_IMAGE`, and the BDD harness disable automatic
graphics detection. The viewer displays the reason. For troubleshooting on a
terminal with known support, set `TESHI_IMAGE_PROTOCOL=sixel` (or `kitty` or
`iterm2`) before launching; `none` disables images explicitly.
Forcing a protocol does not bypass a multiplexer or console backend that
filters images. Native Windows Zellij launches show an explicit compatibility
hint; a direct Windows Terminal session is a separate acceptance target.

Tree nodes whose scenario has captures end with `●N`, where N is the number of
step images in the newest run. The Explore Scenarios and Steps panes use the
same marker, counted per step there. Markers are refreshed when the tab is
opened, when a Feature is loaded, when a run ends, and whenever the viewer
opens. Selecting a scenario without captures lists the other scenarios in the
project that have them.

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
| Ctrl+U / Ctrl+D | Scroll half a viewport up / down |
| `gg/G`, Home/End | History top/bottom |
| PageUp/PageDown, Ctrl+B/Ctrl+F | Scroll history |
| Ctrl+G | Show or hide the AI tab Chats sidebar, also while typing |
| `i` / Enter | Focus input |
| Ctrl+Shift+C | Copy the chat selection |
| Esc (AI tab, request running) | Withdraw the newest queued follow-up, or ask before interrupting the turn |
| Esc (AI tab, agent idle) | Ask to quit Teshi |

Only on the AI tab: `[`/`]` select the previous/next Agent conversation,
`a` creates one, and `x` closes the current one. `j/k` no longer switch Agents.
**Ctrl+W h** moves focus from the chat to the Chats sidebar and **Ctrl+W l**
moves back (the chord stops at the edges like other pane rows). A focused
sidebar shows a highlighted border and `j/k` selects the previous/next
conversation; Enter, `l`, or `i` returns to the composer and Esc returns to the
chat without focusing input. Clicking a conversation focuses the sidebar too.

### Input

| Key | Action |
|-----|--------|
| Printable characters | Insert text |
| Enter | Send; empty input returns to Normal |
| Ctrl+J / Shift+Enter | Insert a newline |
| Left/Right, Up/Down | Move input cursor; Up/Down move between input lines, then on the first/last line recall earlier/later prompts sent in this conversation (Down past the newest restores the draft) |
| Home/End | Move to the start/end of the current input line |
| Backspace/Delete | Delete text; an `[Image #N]` placeholder is deleted as a whole |
| Ctrl+V / Alt+V | Paste clipboard text; with no text on the clipboard, attach its image or image files. In Windows Terminal, Ctrl+V reaches Teshi as the terminal's empty paste for an image-only clipboard; Alt+V reads the clipboard directly in any terminal |
| Alt+arrows | Scroll history |
| PageUp/PageDown, Ctrl+K | Scroll history vertically |
| Ctrl+Home/End | History top/bottom |
| Ctrl+Shift+C | Copy chat selection |
| Esc | Return to Normal |

In the AI tab, Esc from Input returns to Normal; Esc there withdraws the newest
queued follow-up back into the composer, or opens an "Interrupt the current
run?" confirmation when nothing is queued. Enter or `y` interrupts, `n` or Esc
keeps the turn running, so a second Esc never stops the run by itself. A
confirmed interrupt ends the turn and keeps the ACP process and session for the
next message. With no run in progress, Esc opens the quit confirmation: Enter or
`y` exits, `n` or Esc stays. Ctrl+Q exits from any mode, and `:` then `q` reaches
the same exit without a control chord; Ctrl+C copies the selection instead.
Terminal support is needed to distinguish
Shift+Enter from Enter; Ctrl+J also works in terminals without that support.

A pasted screenshot or a pasted or dropped image file appears in the input as
an `[Image #N]` placeholder; typing `@path` to a PNG, JPEG, GIF, WebP, or BMP
file also attaches it. Pasted screenshots are saved in the user cache, not the
project. Each attachment shows as a `▣` chip above the input; click a chip to
remove it. The agent receives the images as image blocks and the message text
without their placeholders or paths. Images are sent only to ACP agents that
accept them; otherwise the send is blocked and the draft stays. Sent images
appear in the transcript under the message, as wide as the message and up to 16
rows tall; click the `▣` row to open one in the system viewer.

Drag over chat text to select it; Ctrl+Shift+C or right-click copies the displayed
plain text without role rails or soft-wrap line breaks. Scrolling/resizing keep
valid selections; changed streamed text or a different session clears them.

## Requirements

### Tree and linked test points

| Key | Action |
|-----|--------|
| `j/k`, Down/Up | Next/previous row, including folder and query rows |
| `h`, Left | Collapse the selected row; press again to walk up to its parent |
| `l`, Right | Expand the selected row; press again to step into its first child |
| Tab/BackTab, or Ctrl+W then `h/l` or Left/Right | Previous/next pane |
| Home/End | First/last item |
| PageUp/PageDown, Ctrl+B/Ctrl+F | Page through items |
| Enter | Expand/collapse a folder or query; open a requirement in the editor |
| Space | Action menu for the selected row (folders, queries, membership, and the keys below) |
| Ctrl+N | New requirement; with a folder selected, it is filed there on first save |
| `d` / Delete | Open global-document deletion confirmation |
| `i` | Iteration filter |
| `o` | Cycle Current project / All projects / No project |
| `g` | Switch view: Folders (Current project) or Path, and Iteration |
| `H` | Show/hide implemented and verified requirements (Current project only) |
| `b` | Link/unlink the saved document to/from the current project |
| `I` | Edit iteration |
| `p` | Edit optional priority (Unset, P0-P3) |
| `s` / Ctrl+S | Save |

`h`/`l` and Left/Right work like the MindMap tree: one press collapses or expands
the selected row, and repeating the key climbs to the parent or steps into the first
child, so they walk the whole hierarchy. Change panes with Tab/BackTab or Ctrl+W then
`h/l`/Left/Right. The tree reserves `g` for switching views, so use Home/End for its
boundaries. Priority changes preserve document order, Markdown, and unsaved text;
existing documents remain unset. The tree title lists scope, view, iteration, and
(in Current project) the completed filter. See `doc/requirement-organization.md` for
the action menu.

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

Outside Insert, a line with `![alt](path)` is followed by the image. Relative
paths resolve from the document's folder, then from the project root; a missing
file shows a `▣ Image not found` row. The cursor skips image rows, and Insert
shows only the Markdown source.

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
| Interrupt confirmation | h/l or Left/Right select a button; Enter/y interrupts; n/N/Esc keeps the turn running; the prompt closes by itself when the turn ends |
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
| `/copy-code [N]` | Copy block N (default 1) from the last assistant answer |
| `/details` | Toggle full thinking and tool details |
| `/models` | Native model profiles or ACP model settings |
| `/sessions`, `/sessions legacy` | Project conversations / explicitly unassigned legacy history |
| `/project`, `/project <directory>` | Project picker / direct project switch |
| `/approval` | ACP agent approval settings; native backend approval modes |
| `/approval local` | Teshi permission-response policy (Manual/Auto/Bypass) |
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


## Compact panes and external Feature refresh

- **F6** requests a Feature rescan. Automatic directory discovery runs at most
  once a second using the opened project's recursion scope. Running cases and
  pending agent edits defer synchronization. Dirty changes/deletions prompt;
  keep-local retains edits, reload accepts the disk version (including deletion).
- **Shift+Left / Shift+Right** pans the focused Explore pane four columns without
  moving focus. Existing scrollbar pointer controls remain available.
- **j / k** in Step changes selects individual additions/removals. Removed Steps
  are read-only; ambiguous modified text is not used to guess an edit target.
- Requirements, Testpoints and Explore use two columns with vertically combined
  details. Below 70 columns or 12 body rows only the focused pane is shown; normal
  pane navigation still reaches every pane. AI hides its chat sidebar below 90
  columns or when the conversation would drop under 50 columns; **[ / ]** still
  switches conversations at these widths, and **Ctrl+G** collapses the sidebar
  on demand.
- The input focus border no longer adds a transient shortcut title. **Enter**
  sends; **Ctrl+J / Shift+Enter** inserts a newline, as before.

These changes have no PTY acceptance evidence yet.


## Chat profile title and ACP status row

The built-in fallback no longer adds a `Default` suffix to AI Chat. Custom profiles,
including a custom profile named Default, display `Agent: <name>`; `/agent-profile`
still opens selection. Agent/model controls and live metrics share a dedicated row
above the input box. Progress stays in the transcript; approval, errors and task
recovery have their own alert row. On narrow screens, Agent names are clipped and option chips may be omitted; the settings chip
(`⋯` when compact) or **Ctrl+,** opens the full settings picker. Errors, approval and
interrupted-task recovery retain priority.

## Chat activity and follow-up messages

Completed turns automatically collapse their execution process, keeping the question
and final answer visible. Click a process header to expand just that turn. Its summary
counts reads, searches, edits and commands; failed and interrupted calls remain in the
summary even at narrow widths. Running, interrupted and unfinished turns stay open.
`/details` toggles all process and reasoning details in the current conversation.

Readable reasoning streams in a single line with a moving highlight and a preview of
the latest text. Click the `Thinking` label to open the full reasoning body; clicking
it again collapses it. These controls preserve the input focus. When a running turn
has no text or tool animation to show, a breathing dot appears where its next output
will arrive. Neither the wait indicator nor process controls enter copied messages.

The input remains editable during a response or approval wait. **Enter** queues a
follow-up, and successful completion submits queued messages in order. Failed or
cancelled turns pause the queue; `/sendqueued` resumes it, and `/unqueue` returns the
latest queued message to the input for editing. Unsent messages survive session
checkpoints and are paused on restore. Queuing never approves tools or test points.

The configuration row shows an approximate live output rate, such as `~42 tok/s`, while
text or readable reasoning arrives. The estimate uses recent text, disappears during
tool execution or after a streaming pause, and remains separate from provider-reported
token usage and billing. Agents that omit tool details retain their original title;
Teshi does not infer tool categories from free-form titles.

## AI chat presentation

The chat frame stays neutral while the focused input uses the purple outline.
User messages have a subtle pink surface; answers retain a green role label, and the
newest finished answer also opens on a green `▌` bar. Thinking, tools and replies
still streaming keep the quiet `│` guide. Thinking, tools and secondary metadata remain muted. Code-block
and text-selection colors retain their own meaning within a user message.

The chat sidebar takes a quarter of the AI tab, clamped between 20 and 30
columns, and a subdued selected-row fill. It disappears below 90 columns so the
conversation keeps its reading width, and **Ctrl+G** (or the footer Chats
control) hides it on wider terminals; the chat title then reads "chats hidden
[Ctrl+g]". Once a conversation is saved, the title also shows its local session ID;
click the ID to copy it. On narrow panels the ID is kept while the agent name and
sidebar hint are dropped. Agent selection, model selection and overflow settings remain
available above the composer. The
time, context and optional token totals use complete fields rather than clipped
numbers. A compact context percentage preserves the warning in narrow layouts.

The input border contains no statistics. The visible `Send` or `Queue` button uses
the same action as Enter; very narrow inputs retain keyboard submission. Clicking
an empty button focuses the input without sending an empty message. Footer
shortcuts retain their existing actions without repeating tool progress.
