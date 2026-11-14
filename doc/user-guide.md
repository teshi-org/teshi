# User Guide

## Opening a project

Run `teshi` in the project directory to recursively load `.feature` files from
`./features`. When `features` has language directories such as `zh-CN` and
`en-US`, only the one matching your system UI language is loaded. Without
language directories, all Feature files under `features` are included. A missing
`features` directory falls back to `.feature` files directly in the project root,
without scanning subdirectories or creating `features`. If none exist, the project
opens empty. A missing language match reports an error.

Use `teshi .` for the same loading rules, or `teshi project/` to open another project.
Use `teshi -R` or `teshi project/ -R` to recursively scan the entire target directory.
See [CLI startup rules](cli-usage.md#open-files-tui) for details.

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
| `submit_requirements` | Record gathered requirements / document sources; advances to test-point proposal (Strict) or Scenario Planning (Direct) |
| `propose_test_points` | Persist Proposed non-Gherkin test points and pause for human review (Strict policy only) |
| `generate_plan` | Record a scenario plan from **approved** test-point IDs (Strict) or with empty/omitted `test_point_ids` (Direct) |
| `insert_scenario` | Insert a new scenario (queues for user approval; embeds `@teshi-tp:<id>` tags when IDs exist) |
| `update_step` | Update a step body (queues for user approval) |

Editing tools queue file changes for your approval: `Y` to accept, `N`/`Esc` to reject, `D` to view a diff.

**Human test-point review is an optional policy, not a fixed gate.** Two review policies apply per generation session:

- **Strict** (default) — test points are proposed, then a human approves them in the Test Points tab before planning. `ApprovalMode` Auto/Bypass only affects file-change queues; it never approves test points or skips Reviewing Test Points.
- **Direct** — the user explicitly authorizes no-review generation. The pipeline goes straight from gathering to planning and writing. No test points are persisted, scenarios may omit `test_point_ids`, no `@teshi-tp:<id>` tags are emitted, and no approval records are fabricated. The chosen policy and the authorization basis are stored in `.teshi/generation-state.json`, so a resumed session does not fall back into the review gate; one authorization stays effective for its task scope. This is recorded as **user-authorized no-review**, never as **human approved**. Existing test points and review records are left untouched.

Enter Direct with an explicit choice: `/generate-direct`, or a recognized chat instruction such as "generate directly" / "skip review". Return to the default with `/review-strict`. A fresh `/generate` also starts in Strict.

Slash helpers:

- `/generate` — confirm the requirement-library source scope, then start requirements gathering under the default Strict policy
- `/generate-direct` — confirm the source scope and start a user-authorized no-review (Direct) generation
- `/review-strict` — revoke a Direct authorization and return to the Strict review policy
- `/continue` — continue generation after approving test points (same as `c` on the Test Points tab; Strict only)
- `/rename <title>` — set a custom conversation title; automatic titles pause while it is set
- `/rename --clear` — drop the custom title and resume automatic titles

Conversation titles are generated after a normally finished turn: the panel header, the chat
sidebar and `/sessions` show the same label. An ACP agent's `session_info_update` title outranks the
generated one, and a manual `/rename` outranks both. Cancelled or failed turns never rename the
conversation. Titles follow the language of the user's messages: a Chinese conversation gets a
Chinese title, and a pure-English title returned for a Chinese conversation is discarded in favour
of the first-message label.

The AI tab is hidden when no LLM credentials are configured. Press `m` to manage shared model profiles, or type `/auth` for a status overview. From the shell, use `teshi auth login` (same store as Desktop Settings).

### Requirements

Authoring tab for the **user-level requirement library** shared by all projects. The library is ordinary Markdown plus `_teshi.json`; it is not stored in the Git repo of the project you have open.

Default locations:

| Platform | Path |
|----------|------|
| Windows | `%APPDATA%\teshi\requirements\` |
| Linux | `$XDG_DATA_HOME/teshi/requirements` (often `~/.local/share/teshi/requirements`) |
| macOS | `dirs::data_dir()/teshi/requirements` |

Override for this process with `teshi --requirements-root PATH`, or persistently with `TESHI_REQUIREMENTS_DIR`. `TESHI_APP_DATA_DIR` still changes the Teshi app-data root (and therefore the default `requirements/` subdirectory). Confirm the resolved path with `teshi requirements path`. The library can live on another disk or in its own Git repository; Teshi does not sync it for you.

`<project>/requirements/` is **not** loaded at runtime. If Teshi finds that legacy directory, it shows a migration diagnostic. Preview with `teshi requirements import-project --dry-run`, then import (IDs/paths are kept or remapped; project test-point links get the target `store_id`; source files are left in place).

- **Tree** — defaults to documents associated with the current project. Press `o` in the tree to cycle Current project / All projects / No project, and `b` to link or unlink the selected saved document. The title shows both the project window and iteration filter. Press `i` to filter by All / Unassigned / a named iteration, `g` to group by path or iteration, `I` to set the selected document's iteration (empty = Unassigned). Iteration and group preferences are saved per library and project in user settings. Opening a project always starts with its Current project window.
- **Editor** — Markdown body with range selection and linked-range highlights
- **Linked test points** — filtered by the active selection when present; test points themselves stay in `<project>/testpoints/testpoints.json`

Select text and press `n` to create a `Proposed` test point linked to that exact range (identity is `(store_id, document_id)`). Press `Ctrl+n` for a new document, automatically associated with the current project. Older documents remain unassigned until explicitly linked; use All projects or No project to find them. Unlinking keeps the shared document and other project associations. Delete still removes the document from the shared library for every project.

To rename a requirement, edit its first line (`# Title`) and press `Ctrl+s`.
Saving synchronizes the name in the Requirements tree while preserving the
document ID, file path, and test-point links. An empty title or a first line
without an H1 heading prevents saving and leaves the existing document intact.

`/generate` asks you to confirm the source scope (store path, store ID, iteration, project window). The confirmed scope is fixed for that generation session; subsequent view changes do not change it. Agent reads recheck live project and iteration membership. Pasted chat text is not a requirement-library source.

Agents and scripts should use the same store through `teshi requirements` rather than editing `_teshi.json` directly:

```bash
teshi requirements list --iteration "Sprint 12" --json
teshi requirements show doc-12
teshi requirements set-iteration doc-12 doc-13 --iteration "Sprint 13"
teshi requirements edit doc-12 --file body.md
```

Refs may be a document ID, relative path, or unique title. `edit` without `--file` opens `$VISUAL` / `$EDITOR` (Windows falls back to `notepad`). See [cli-usage.md](cli-usage.md) for the JSON envelope, revision guard, and `--force`.

Project association, per-project delivery states, and portable snapshots are documented
in [Requirements shared across projects](requirement-projects.md).

### Test Points

Review tab for non-Gherkin verification intents stored in `testpoints/testpoints.json`:

- **Tree** — business hierarchy with review-state indicators (`f` cycles filters)
- **Details** — title, objective, preconditions, expected outcomes, hierarchy, review state, and realized scenarios
- **Excerpts** — linked requirement ranges (`Enter` opens Requirements at the highlight)

Review actions: `a` approve, `A` batch approve, `r` reject. After at least one eligible approval, press `c` (or `/continue`) to advance the AI pipeline to Scenario Planning. Press `o` to open a realized Gherkin scenario. These review actions belong to the **Strict** policy; a **Direct** (no-review) generation does not pause here and cannot be mistaken for human approval.

Review states: `Proposed` → `Approved` / `Rejected`; approved points with stale anchors become `NeedsReview`.

## Feature generation pipeline

When you ask the AI to create a feature (including `/generate`), the flow depends on the active review policy. The system prompt, stage guidance, tool descriptions, and the persisted generation state all follow the same policy.

**Strict** (default):

1. **Requirements Gathering** — confirm store/iteration scope, then conversational paste and/or persisted document/range sources via `submit_requirements` (`list_requirement_documents` / `read_requirement_document` read the current store on demand)
2. **Generating Test Points** — agent calls `propose_test_points` (no Given/When/Then inside test points)
3. **Reviewing Test Points** — agent pauses; humans approve in the Test Points tab
4. **Planning** — `generate_plan` with approved, resolved `test_point_ids`
5. **Writing** — `create_feature_file` / `insert_scenario` write Gherkin; scenarios keep `@teshi-tp:<id>` tags
6. **Validation** — `validate_feature` / tests as applicable

**Direct** (user-authorized no review):

1. **Requirements Gathering** — same as Strict
2. **Planning** — `generate_plan` directly; `test_point_ids` may be empty or omitted, and no IDs or approvals are invented
3. **Writing** — `create_feature_file` / `insert_scenario` write Gherkin; with no IDs, no `@teshi-tp:<id>` tags are added
4. **Validation** — `validate_feature` / tests as applicable

`propose_test_points` is unavailable under Direct, and the pipeline never waits in the Test Points tab. Traceability distinguishes **human approved** (Strict with approvals), **user-authorized no-review** (Direct), and **not reviewed**; a Direct generation is never labeled human approved. Traceability to test points can be added later without rewriting history.

Restarting the TUI restores the review phase, the chosen review policy and its authorization basis, confirmed source scope, and artifacts from `.teshi/generation-state.json` without implicitly approving anything. A Direct session resumes at Planning and never falls back into the review gate; a Strict session without approvals still waits. A store mismatch, missing iteration, or document revision drift pauses generation until you reconfirm sources. Legacy sessions that referenced documents without a store identity are not rebound to the current library; legacy state files without a policy field default to Strict.

### Review policy design decisions

doc-48 leaves the policy's configuration level, default, scope, and revocation to design time. The chosen decisions are:

- **Configuration level** — a per-generation-session value in `.teshi/generation-state.json`, not a global or always-on project setting. Strict remains the default so review is not silently disabled for all projects.
- **Default** — `Strict`. A fresh `/generate`, a new session, or a legacy state file without a policy field all start in Strict.
- **Scope** — one explicit authorization is effective for its current task scope (the feature generation it was granted for) and is persisted with a `basis` (the recognized instruction or command) and a `scope` (the originating request). A project switch is treated as a scope change and resets the policy to Strict. There is no cross-project or permanent disable.
- **Revocation** — `/review-strict` revokes a Direct authorization and returns to Strict; a fresh `/generate` does the same. Switching back mid-generation requires re-running the Strict review flow.
- **Traceability** — `ReviewPolicy` records the strategy, `DirectAuthorization` records the basis, and `review_traceability` classifies a generation as `human approved`, `user-authorized no-review`, or `not reviewed` without ever granting approval. Existing test points and review records are never reset, overwritten, or impersonated.

## External Test Runner

The `teshi run` subcommand executes BDD feature files through runners declared in `teshi.toml`. Each scenario must match exactly one `[[runners]]` rule, so one project can mix the built-in API dispatch, the TUI runner and your own adapters.

```bash
teshi run tests/features/editor.feature
teshi run --explain tests/features/     # which runner handles each scenario
```

```toml
[[runners]]
name = "editor"
cmd = "cargo"
args = ["run", "--bin", "teshi-runner"]
[runners.match]
paths = ["tests/features/**/*.feature"]
```

External runners speak NDJSON protocol v1 and must open with a `start_run` handshake. Results are displayed inline in the Explore tab with status colors per scenario and step; scenarios whose target was unavailable show as **Blocked** rather than failed. See [runner-protocol.md](runner-protocol.md).

See [CLI & Config](cli-usage.md) for more runner options. Requirement CLI control-plane E2E uses this same protocol (`@cli` files in `features/en-US/` or `features/zh-CN/` + `teshi-requirement-cli-runner`).

## HTTP API BDD

Gherkin can drive HTTP APIs. Each interface is one Jinja2 file under `api/` (override with `[api] templates` in `teshi.toml`). Python step defs in `features/steps/` call `call("create_user.json.j2")` — variables flow through scenario `vars`, not kwargs.

- Mark HTTP steps with `[API]` immediately after the keyword (`When [API] I create a user named "Ada"`).
- Tag scenarios `@api`, `@ui`, or both (`@api @ui` for mixed). Scenario engine tags override Feature tags (no union).
- Mixed and Explore runs use Teshi step dispatch: `[API]` → API sidecar, other steps → browser/WinApp bindings. Pure `@api` CI can use behave with the same helper (`TESHI_API_NDJSON=1`).
- TUI Explore shows redacted `http_exchange` events; press `p` in the detail pane to expand plaintext (requires a running sidecar). GPUI desktop/web **Run** is inspect-only — it does not edit `.feature` files.

Seed vars from `[api]` keys plus `TESHI_API_*` environment variables (`TESHI_API_TOKEN` → `token`). See [CLI & Config](cli-usage.md).

## WinUI3 / Native App Recording

`teshi desktop`/`teshi web` can expose a WinUI3/native Windows app bridge
through the supported control in the installed build. Terminal agents can
then use `teshi winapp` commands through the tracked `winapp-regression` skill.
Do not assume a control named **Connect WinUI3 App** exists in every build.
Confirmed UIA bindings are stored in `.teshi/step-bindings/{feature}.json` with
`strategy: "uia"`.

See [WinUI3 / Native Windows app mode](winapp-modes.md) for setup and limitations.

## Syntax Highlighting

- Gherkin headers (`Feature`, `Scenario`, `Scenario Outline`, `Examples`, `Background`)
- Steps (`Given`, `When`, `Then`, `And`, `But`)
- Tags (`@tag`, including Teshi `@teshi-tp:<id>` links)
- Comments (`# ...`)
- Strings (`"..."`)
- Tables and doc string markers (`|`, `"""`)
