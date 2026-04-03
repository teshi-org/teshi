# teshi

A minimal Rust TUI editor for pure Gherkin (`.feature`) files.

## Features (MVP)

- Open a file from CLI argument: `cargo run -- path/to/file.feature`
- Arrow-key navigation + Space-triggered single-step text editing
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
- `↑/↓/←/→`: move cursor
- `Home` / `End`: move to line start/end
- `PageUp` / `PageDown`: move by 10 lines
- `Space`: activate single-step text input on current step line (`Given/When/Then/And/But`)
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