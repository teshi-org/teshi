# Keybindings

## Tab switching

| Key | Tab |
|-----|-----|
| `1` | Explore |
| `2` | MindMap |
| `3` | AI |
| `4` | Requirements |
| `5` | Test Points |

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

The Markdown editor starts in **Browse** mode. Press `i` in the editor to enter
**Insert**, and `Esc` to return to Browse. Browse never inserts ordinary text.

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Next / previous pane in Browse |
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
Bracketed paste accepts text in Insert. Use `Esc`, then `Tab`, to change panes.
Mouse dragging selects Markdown text for creating a test point in Browse.

Leaving a dirty document through document selection, a filter that hides it,
main-tab navigation, new-document creation, or quit opens an unsaved-changes
prompt: `S` saves and continues, `D` discards and continues, and `Esc` cancels.
A failed save keeps the buffer and prompt open so the operation can be retried.

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

## Structural editing

| Key | Action |
|-----|--------|
| `Ctrl+/` | Undo (full buffer snapshot) |
| `Ctrl+Y` | Redo |
| `s` | Save current file |
| `q` | Quit (press twice if buffer is dirty) |
