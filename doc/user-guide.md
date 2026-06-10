# User Guide

## Tabs

teshi has three core tabs:

### Explore

Three-column browser: **features** → **scenarios** → **steps**.

- **Features column** — list of `.feature` files. `j`/`k` or `↑`/`↓` to move; `e` to enter the editor for the selected file.
- **Scenarios column** — scenarios within the selected feature. Shows test run status (pending / running / passed / failed / skipped). `r` to run the selected scenario.
- **Steps column** — steps of the selected scenario with test case status. `Enter` toggles failure detail on failed steps.

Column navigation: `Tab` / `→` to move right, `BackTab` / `←` / `h` to move left.

### MindMap

Interactive tree view of all scenarios and steps, with highlights, filters, and cross-file step reuse detection.

Three-stage layout:

- **Tree panel** (left) — collapsible tree of features → scenarios → steps. `Enter` expands/collapses nodes.
- **Editor panel** (right, stage 2) — read-only preview of the selected node's source lines. Available when a non-root node is selected.
- **Step body panel** (right, stage 3) — editable step body for the selected step line.

Highlights and filters available via AI tools (`highlight_mindmap_nodes`, `apply_mindmap_filter`).

### AI

Chat interface with an LLM-powered function-calling agent. The agent can inspect the project and queue edits for your approval.

| Tool | Description |
|------|-------------|
| `get_project_info` | Project overview: feature files, scenario/step counts, active file |
| `get_feature_content` | Full parsed content of a `.feature` file |
| `highlight_mindmap_nodes` | Highlight MindMap nodes matching a condition |
| `apply_mindmap_filter` | Filter the MindMap tree by node name |
| `insert_scenario` | Insert a new scenario (queues for user approval) |
| `update_step` | Update a step body (queues for user approval) |

Editing tools queue changes for your approval: `Y` to accept, `N`/`Esc` to reject, `D` to view a diff.

The AI tab is hidden when no LLM credentials are configured. Type `/auth` in the chat to manage credentials.

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

See [CLI & Config](cli-usage.md) for more runner options.

## WinUI3 / Native App Recording

`teshi desktop` can start a WinUI3/native Windows app bridge with **Connect WinUI3 App**. Terminal agents can then use `teshi winapp` commands through the tracked `winapp-locator` skill. Confirmed UIA bindings are stored in `.teshi/step-bindings/{feature}.json` with `strategy: "uia"`.

See [WinUI3 / Native Windows app mode](winapp-modes.md) for setup and limitations.

## Syntax Highlighting

- Gherkin headers (`Feature`, `Scenario`, `Scenario Outline`, `Examples`, `Background`)
- Steps (`Given`, `When`, `Then`, `And`, `But`)
- Tags (`@tag`)
- Comments (`# ...`)
- Strings (`"..."`)
- Tables and doc string markers (`|`, `"""`)
