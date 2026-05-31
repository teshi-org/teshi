# teshi

Terminal-first BDD editor with AI assistance, mind-map navigation, and external test runner integration.

## Quick Start

```bash
cargo run                              # TUI: scan current directory for .feature files
cargo run -- path/to/features/         # open a directory of .feature files
cargo run -- path/to/file.feature      # open a single feature file
cargo run -- web --project path/       # browser GUI (loopback HTTP server)
cargo run -- desktop --project path/   # native desktop shell (locator workflow)
cargo run -- run path/to/file.feature  # headless BDD test run (NDJSON runner)
```

No `.feature` files in the current directory opens an empty project buffer.

### CLI Commands

| Use case | Command |
|----------|---------|
| Terminal editing + AI | `teshi` / `teshi path/` |
| Browser GUI (lightweight) | `teshi web [--project PATH]` |
| Native desktop (Chrome locator) | `teshi desktop [--project PATH]` or `teshi-desktop [--project PATH]` |
| Headless CI runs | `teshi run [PATH] [--scenario NAME]` |
| Credentials | `teshi auth login \| list \| remove \| status \| migrate` |

```bash
teshi auth login [--provider <name>]   # Store an API key for a provider
teshi auth list                        # Show stored providers (keys masked)
teshi auth remove <provider>           # Delete stored credentials
teshi auth status                      # Show config and credential status
teshi auth migrate                     # Migrate API keys from env vars to auth.json
teshi run [path] [--scenario NAME]     # Run BDD tests (default path: current directory)
teshi web [--project PATH]             # Browser GUI via loopback server
teshi desktop [--project PATH]         # Launch native desktop shell
```

## Tabs

| Key | Tab | Purpose |
|-----|-----|---------|
| `1` | Explore | Three-column browser: features → scenarios → steps. Navigate, edit, run, and AI-suggest. |
| `2` | MindMap | Interactive tree view of all scenarios and steps, with highlights, filters, and cross-file step reuse detection. |
| `3` | AI | Chat interface with function-calling LLM agent that can inspect the project and queue edits for your approval. |

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

Type `/auth` in the chat to open the credential management panel (provider overview, key status, add/remove).

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

## Syntax Highlighting

- Gherkin headers (`Feature`, `Scenario`, `Scenario Outline`, `Examples`, `Background`)
- Steps (`Given`, `When`, `Then`, `And`, `But`)
- Tags (`@tag`)
- Comments (`# ...`)
- Strings (`"..."`)
- Tables and doc string markers (`|`, `"""`)

## Environment Variables

### LLM / Provider

You can use environment variables directly, or use the config file + credentials system (see [Configuration](#configuration) below).

| Variable | Required | Default | Description |
|---|---|---|---|
| `TESHI_DEFAULT_PROVIDER` | No | — | Provider name to use (e.g. `deepseek`) |

Legacy env vars (still supported, prefer `teshi auth login`):

| Variable | Required | Default | Description |
|---|---|---|---|
| `TESHI_LLM_API_KEY` | Yes (legacy) | — | API key for the LLM provider |
| `TESHI_LLM_BASE_URL` | No | `https://api.openai.com/v1` | OpenAI-compatible API base URL |
| `TESHI_LLM_MODEL` | No | `gpt-4o-mini` | Model name to use |
| `TESHI_LLM_MAX_TOKENS` | No | `1024` | Max tokens per completion |
| `TESHI_LLM_TEMPERATURE` | No | `0.7` | Sampling temperature |

The AI tab is hidden when no LLM credentials are configured.

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

## Configuration

teshi supports layered configuration with placeholder resolution.  
Config loading order: hardcoded defaults → `~/.teshi/config.toml` → `.teshi/config.toml` → environment variables (highest priority).

### `~/.teshi/config.toml` (user-level)

```toml
default_provider = "deepseek"

[providers.deepseek]
base_url = "https://api.deepseek.com"
model = "deepseek-chat"
api_key = "${auth:deepseek}"

[providers.openai]
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
api_key = "${auth:openai}"
```

### `api_key` placeholder formats

| Format | Source |
|--------|--------|
| `${auth:provider}` | Read from `~/.teshi/auth.json` |
| `${env:VAR}` | Read from environment variable |
| Plaintext `"sk-..."` | Used directly (not recommended) |

### `~/.teshi/auth.json` (credential store)

Created and managed by `teshi auth login` / `teshi auth remove`.  
Stored with `0600` permissions on Unix. Never commit this file.

### Project-level `.teshi/config.toml`

Override model or `base_url` per-project. Do not store API keys here.

The `teshi.toml` config file in the working directory supports:

```toml
[runner]
command = "cargo"
args = ["run", "--bin", "teshi-runner"]
cwd = "."          # optional working directory
```

## Contributing

### Getting started

Clone the repository and enable the git hooks:

```bash
git config core.hooksPath .githooks
```

This runs automated checks (formatting, compilation, `dbg!()` guard, commit message convention) on every commit, and lint + tests + doc build on every push.
