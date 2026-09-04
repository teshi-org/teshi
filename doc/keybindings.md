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

| Key | Action |
|-----|--------|
| `Tab` | Cycle tree / editor / linked test-point panes |
| `n` | Create a Proposed test point from the active non-empty selection |
| `Ctrl+n` | Create a new requirement Markdown document |
| `i` | Filter the tree by iteration (All / Unassigned / named) |
| `g` | Toggle grouping: path hierarchy vs iteration → path |
| `I` | Edit the selected document's iteration (empty = Unassigned) |
| `s` | Save the current requirement document |

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
