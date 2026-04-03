# teshi

A minimal Rust TUI editor for pure Gherkin (`.feature`) files.

## Features (MVP)

- Open a file from CLI argument: `cargo run -- path/to/file.feature`
- BDD-aware navigation (↑/↓ between nodes, ←/→ keyword vs body) and Space-triggered step editing
- Gitui-style layout with switchable top tabs and dynamic bottom hints
- Syntax highlighting for:
  - Gherkin headers (`Feature`, `Scenario`, `Scenario Outline`, `Examples`, `Background`)
  - Steps (`Given`, `When`, `Then`, `And`, `But`)
  - Tags (`@tag`)
  - Comments (`# ...`)
  - Strings (`"..."`)
  - Tables and doc string markers (`|`, `"""`)

## Run

```bash
cargo run -- examples/demo.feature
```

If no file path is passed, the editor starts with an empty buffer.

## Keybindings

- `1` / `2` / `3`: switch top tabs (`Editor` / `Feature` / `Help`) when step input is inactive
- `↑` / `↓` (Editor, not in overlays): jump to the previous/next BDD node (headers + steps)
- `←` / `→` (Editor, not in overlays): toggle focus between the step keyword and the step body (step lines only)
- `Home` / `End` (Editor, not in overlays): jump to the first/last BDD node
- `PageUp` / `PageDown` (Editor, not in overlays): jump about 10 nodes backward/forward
- `Space` (Editor): on **keyword** focus, open the step-keyword dropdown; on **body** focus, start step text input (`Given` / `When` / …)
- `Enter`: commit current step text input
- `Esc`: clear current input state
- `s`: save current file
- `q`: quit (press twice if buffer is dirty)
- While step input is active: printable chars / `Backspace` / `Delete` edit only text after the step keyword

## Tabs

The top bar labels match shortcuts: `Editor [1]`, `Feature [2]`, `Help [3]`.

- `Editor`: editable `.feature` content with syntax highlighting
- `Feature`: read-only outline panel (`Feature`, `Scenario`, `Scenario Outline`, `Examples`)
- `Help`: quick in-app keybinding reference

## Known Limitations (MVP)

- No file picker (use CLI argument)
- No undo/redo
- No multi-tab or split view
- No test framework integration or step completion