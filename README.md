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
- `↑` / `↓` (Editor, not in overlays): with **keyword** focus, previous/next navigable line (headers, `Feature:` description lines, steps); with **body** focus on a step or editable header title (not feature prose), previous/next line in document order among **steps** and editable titles (`Feature:` / `Scenario:` / `Scenario Outline:` / `Examples:`); **body** on a `Feature:` description line uses the same rule as keyword (all navigable lines)
- `←` / `→` (Editor, not in overlays): toggle between the Gherkin keyword/token and the editable text after it (step bodies; `Feature:` / `Scenario:` / `Scenario Outline:` / `Examples:` titles; not `Background:`); free-text lines under `Feature:` use **body** only (whole line)
- `Home` / `End` (Editor, not in overlays): first/last BDD node (keyword focus) or first/last entry in the body chain above (same body-focus rule as `↑` / `↓`)
- `PageUp` / `PageDown` (Editor, not in overlays): about 10 BDD nodes or body-chain lines, matching the same rule as `↑` / `↓`
- `Space` (Editor): on **keyword** focus, open the step-keyword dropdown on step lines (not on headers); on **body** focus, start editing the step body or header title after the colon
- `Enter`: commit the active line edit
- `Esc`: clear current input state
- `s`: save current file
- `q`: quit (press twice if buffer is dirty)
- While line edit is active: printable chars / `Backspace` / `Delete` only change text after the step keyword or after the editable header prefix

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