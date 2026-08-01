# User Guide

## Tabs

teshi has five core tabs:

### Explore

Three-column browser: **features** → **scenarios** → **steps**.

- **Features column** — list of `.feature` files. `j`/`k` or `↑`/`↓` to move; `e` to enter the editor for the selected file.
- **Scenarios column** — scenarios within the selected feature. Shows test run status (pending / running / passed / failed / skipped). Linked test-point IDs appear as compact `[tp-…]` badges when scenarios carry `@teshi-tp:<id>` tags. `r` to run the selected scenario.
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
| `submit_requirements` | Record gathered requirements / document sources; advances to test-point proposal |
| `propose_test_points` | Persist Proposed non-Gherkin test points and pause for human review |
| `generate_plan` | Record a scenario plan from **approved** test-point IDs only |
| `insert_scenario` | Insert a new scenario (queues for user approval; embeds `@teshi-tp:<id>` tags) |
| `update_step` | Update a step body (queues for user approval) |

Editing tools queue file changes for your approval: `Y` to accept, `N`/`Esc` to reject, `D` to view a diff.

**Test-point approval is a separate hard gate.** `ApprovalMode` Auto/Bypass only affects file-change queues; it never approves test points or skips Reviewing Test Points.

Slash helpers:

- `/generate` — start requirements gathering
- `/continue` — continue generation after approving test points (same as `c` on the Test Points tab)

The AI tab is hidden when no LLM credentials are configured. Type `/auth` in the chat to manage credentials.

### Requirements

Authoring tab for durable requirement Markdown under `requirements/`:

- **Tree** — indexed documents from `requirements/_teshi.json`
- **Editor** — Markdown body with range selection and linked-range highlights
- **Linked test points** — filtered by the active selection when present

Select text and press `n` to create a `Proposed` test point linked to that exact range. Press `Ctrl+n` for a new document.

### Test Points

Review tab for non-Gherkin verification intents stored in `testpoints/testpoints.json`:

- **Tree** — business hierarchy with review-state indicators (`f` cycles filters)
- **Details** — title, objective, preconditions, expected outcomes, hierarchy, review state, and realized scenarios
- **Excerpts** — linked requirement ranges (`Enter` opens Requirements at the highlight)

Review actions: `a` approve, `A` batch approve, `r` reject. After at least one eligible approval, press `c` (or `/continue`) to advance the AI pipeline to Scenario Planning. Press `o` to open a realized Gherkin scenario.

Review states: `Proposed` → `Approved` / `Rejected`; approved points with stale anchors become `NeedsReview`.

## Feature generation pipeline

When you ask the AI to create a feature (including `/generate`), the authoritative flow is:

1. **Requirements Gathering** — conversational paste and/or persisted document/range sources via `submit_requirements`
2. **Generating Test Points** — agent calls `propose_test_points` (no Given/When/Then inside test points)
3. **Reviewing Test Points** — agent pauses; humans approve in the Test Points tab
4. **Planning** — `generate_plan` with approved, resolved `test_point_ids`
5. **Writing** — `create_feature_file` / `insert_scenario` write Gherkin; scenarios keep `@teshi-tp:<id>` tags
6. **Validation** — `validate_feature` / tests as applicable

Restarting the TUI restores the review phase and artifacts from disk without implicitly approving anything.

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
- Tags (`@tag`, including Teshi `@teshi-tp:<id>` links)
- Comments (`# ...`)
- Strings (`"..."`)
- Tables and doc string markers (`|`, `"""`)
