# teshi

Terminal-first BDD editor with AI assistance, mind-map navigation, and external test runner integration.

## Quick Start

```bash
cargo run -- path/to/features/          # open a directory of .feature files
cargo run -- path/to/file.feature       # open a single feature file
cargo run -- run path/to/file.feature   # run BDD tests (NDJSON-based runner)
```

No arguments opens an empty buffer.

## Tabs

| Key | Tab | Purpose |
|-----|-----|---------|
| `1` | Explore | Three-column browser: features → scenarios → steps. Navigate, edit, run, and AI-suggest. |
| `2` | MindMap | Interactive tree view of all scenarios and steps, with highlights, filters, and cross-file step reuse detection. |
| `3` | AI | Chat interface with function-calling LLM agent that can inspect the project and queue edits for your approval. |
| `4` | Help | In-app keybinding reference. |

## Explore Tab

The Explore tab presents project state in a three-column layout:

- **Features column** — list of `.feature` files. `j`/`k` or `↑`/`↓` to move; `e` to enter the editor for the selected file; `a` to open the AI chat with a suggestion prompt for the selected scenario.
- **Scenarios column** — scenarios within the selected feature. Shows test run status (pending / running / passed / failed / skipped). `r` to run the selected scenario.
- **Steps column** — steps of the selected scenario with test case status. `Enter` toggles failure detail on failed steps.

Column navigation: `Tab` / `→` to move right, `BackTab` / `←` / `h` to move left.

## MindMap Tab

Three-stage layout showing the full step hierarchy as a tree:

- **Tree panel** (left) — collapsible tree of features → scenarios → steps. `Enter` expands/collapses nodes; stage-1 keyboard shortcuts apply.
- **Editor panel** (right, stage 2) — read-only preview of the selected node's source lines. Available when a non-root node is selected.
- **Step body panel** (right, stage 3) — editable step body for the selected step line.

Highlights and filters available via AI tools (`highlight_mindmap_nodes`, `apply_mindmap_filter`). Press `Tab` to cycle through MindMap location selections.

## AI Tab

Chat interface with an LLM-powered function-calling agent. The agent has access to six tools:

| Tool | Description |
|------|-------------|
| `get_project_info` | Project overview: feature files, scenario/step counts, active file |
| `get_feature_content` | Full parsed content of a `.feature` file |
| `highlight_mindmap_nodes` | Highlight MindMap nodes matching a condition |
| `apply_mindmap_filter` | Filter the MindMap tree by node name |
| `insert_scenario` | Insert a new scenario (queues for user approval) |
| `update_step` | Update a step body (queues for user approval) |

Editing tools queue changes for your approval: `Y` to accept, `N`/`Esc` to reject, `D` to view a diff. `Esc` toggles between the chat input and message list. `Alt+↑`/`Alt+↓` to scroll chat history.

## Editor Keybindings

### Navigation (Explore / MindMap)

| Key | Action |
|-----|--------|
| `↑` / `↓` / `j` / `k` | Previous / next navigable line or tree node |
| `←` / `→` / `h` / `l` | Toggle keyword vs body focus; move between columns |
| `Home` / `End` | First / last node or line |
| `PageUp` / `PageDown` | Scroll ~10 nodes or lines |

### Editing (in editor / step body mode)

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

### Structural Editing

| Key | Action |
|-----|--------|
| `Ctrl+/` | Undo (full buffer snapshot) |
| `Ctrl+Y` | Redo |
| `s` | Save current file |
| `q` | Quit (press twice if buffer is dirty) |

## External Test Runner

The `teshi run` subcommand executes BDD feature files against a configurable NDJSON-based runner.

```bash
teshi run tests/features/editor.feature
```

Configure the runner command in `teshi.toml`:

```toml
[runner]
command = "cargo"
args = ["run", "--bin", "teshi-runner"]
```

Test results stream back as NDJSON lines and are displayed inline in the Explore tab with status colors per scenario and step.

## Self-Bootstrapping

teshi's own feature matrix is described in BDD feature files under `tests/features/`:

- `editor.feature` — BDD navigation, step editing, keyword picker, syntax highlighting
- `mindmap.feature` — mind map tree view, three-stage layout, step reuse detection
- `project.feature` — single/multi-file loading, Gherkin parsing, edit-to-tree sync

Additional demo files live under `tests/features_demo/`.

## Syntax Highlighting

- Gherkin headers (`Feature`, `Scenario`, `Scenario Outline`, `Examples`, `Background`)
- Steps (`Given`, `When`, `Then`, `And`, `But`)
- Tags (`@tag`)
- Comments (`# ...`)
- Strings (`"..."`)
- Tables and doc string markers (`|`, `"""`)

## Environment Variables

### LLM (AI tab)

| Variable | Required | Default | Description |
|---|---|---|---|
| `TESHI_LLM_API_KEY` | Yes | — | API key for the LLM provider |
| `TESHI_LLM_BASE_URL` | No | `https://api.openai.com/v1` | OpenAI-compatible API base URL |
| `TESHI_LLM_MODEL` | No | `gpt-4o-mini` | Model name to use |
| `TESHI_LLM_MAX_TOKENS` | No | `1024` | Max tokens per completion |
| `TESHI_LLM_TEMPERATURE` | No | `0.7` | Sampling temperature |

The AI tab is hidden when `TESHI_LLM_API_KEY` is not set.

### Runner

| Variable | Required | Default | Description |
|---|---|---|---|
| `TESHI_RUNNER_CMD` | Yes (if no `teshi.toml`) | — | Executable for the test runner |
| `TESHI_RUNNER_ARGS` | No | — | Space-separated args for the runner |
| `TESHI_RUNNER_CWD` | No | current dir | Working directory for the runner |

Env vars take precedence over `teshi.toml` values.

### Diagnostics

| Variable | Purpose |
|---|---|
| `TESHI_DIAG_PATH` | Write diagnostic log to this file path |
| `TESHI_NO_RAW` | Disable raw terminal mode |
| `TESHI_NO_ALT` | Disable alternate screen |

## Config

The `teshi.toml` config file in the working directory supports:

```toml
[runner]
command = "cargo"
args = ["run", "--bin", "teshi-runner"]
cwd = "."          # optional working directory

[llm]
# Not yet supported in toml; use environment variables above.
```
