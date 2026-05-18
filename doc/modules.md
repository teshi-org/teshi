# Module Reference

## `app.rs` — Application orchestrator (~4300 lines)

The core of the application. `App` holds ~60 fields including:

- **Project state**: `project: BddProject`, `step_index`, `mindmap_index`
- **Editor state**: active `buffer`, persistent `buffers`, cursor/scroll, undo/redo stacks, clipboard, scenario folds
- **UI state**: `tree_state`, `view_stage`, `active_tab`, focus slots, keyword picker
- **Runner state**: config, event channel, explore case mappings, run results, failure details
- **LLM state**: messages, input, partial response, agent loop control, tool status, pending changes
- **Mind map UI**: highlight categories, filter, location selection, AI panel visibility/focus

### Key methods

| Method | Purpose |
|--------|---------|
| `handle_action()` | Central dispatcher — 500-line `match action` covering navigation, editing, agent, runner, and global commands |
| `poll_llm_events()` | Drives the agent loop: processes streaming chunks, executes tool calls, manages the confirmation gate for mutations |
| `poll_external_feature_changes()` | Detects disk changes via `FileStamp`, prompts or auto-reloads |
| `switch_to_buffer()` | Saves current buffer state, clones target buffer into active view |
| `accept_agent_change()` | Applies a confirmed agent mutation, refreshes AST and index |
| `feed_agent_tool_result()` | Sends tool execution result back to LLM, continues agent loop |
| Explorer methods | `build_explore_cases()`, `start_explore_run()`, `apply_run_event()`, persist/restore explore memory |
| Editor operations | `insert_step()`, `insert_scenario()`, `delete_current_node()`, `copy_current_step()`, `paste_step()`, `undo()`/`redo()` |

---

## `gherkin.rs` — BDD/Gherkin parser (~527 lines)

**Hand-written** line-by-line state machine. No external parser dependency.

### AST types

- `BddProject { root_dir, features: Vec<BddFeature> }`
- `BddFeature { file_path, name, description, tags, background, scenarios }`
- `BddScenario { name, line_number, tags, kind: ScenarioKind, steps, examples }`
- `ScenarioKind::Scenario | ScenarioOutline`
- `BddBackground { name, line_number, steps }`
- `BddStep { keyword, text, line_number, doc_string, data_table }`
- `BddDocString { content, delimiter, line_number }`
- `ExamplesTable { tags, name, headers, rows, line_number }`

### Parsing logic

The parser recognizes these constructs by line prefix:

| Pattern | Construct |
|---------|-----------|
| `@tag` | Tags (collected for feature and scenario) |
| `Feature:` | Feature header |
| `Background:` | Background block start |
| `Scenario:` / `Scenario Outline:` | Scenario start |
| `  Given/When/Then/And/But` | Step (requires leading whitespace) |
| `"""` | Docstring delimiter |
| `\| ... \|` | Data table / examples row |

Leading whitespace on step keywords distinguishes them from free-text descriptions that happen to start with the same words.

---

## `editor_buffer.rs` — Rope wrapper (~274 lines)

Thin wrapper around `ropey::Rope` providing:

- `line(row) → String`
- `replace_line(row, text)`
- `insert_char(row, col, ch) → (new_row, new_col, changed)`
- `insert_str()` / `backspace()` / `delete()` / `insert_line()`
- `text_range(start, end) → String`
- `clamp_col()` — handles multi-byte character boundaries

Used by both the persistent file buffers and the active editing view.

---

## `mindmap.rs` — Prefix trie index (~695 lines)

### Data structures

- `TrieNode { children: HashMap<String, usize>, locations: Vec<NodeLocation>, category, highlights }`
- `NodeLocation { feature_idx, line_number, scenario_idx, context: LocationContext }`
- `MindMapFilter::NameContains(String)`
- `HighlightRule` with `MatchCondition::StepContains(text, case_sensitive)`

### Key functions

| Function | Purpose |
|----------|---------|
| `build_index(project)` | Inserts all background + scenario steps into a trie arena, deduplicating identical subsequences |
| `rebuild_items_from_arena()` | Converts trie nodes into `TreeItem<String>` for `tui-tree-widget`, applying filters and highlights |
| `compute_node_categories()` | Classifies nodes as `Selected`, `Ancestor`, `Descendant`, `Sibling`, or `GrayedOut` for styling |
| `find_closest_node()` | Maps an editor cursor line to the nearest trie node for editor ↔ tree synchronization |
| `tree_cycle_location()` | Cycles through multiple occurrences of the same step text |

---

## `bdd_nav.rs` — Structured BDD navigation (~573 lines)

Pure functions operating on `EditorBuffer` for BDD-aware editing:

| Function | Purpose |
|----------|---------|
| `bdd_step_rows(buffer)` | Returns indices of all navigable step and header lines |
| `next_node_row()` / `prev_node_row()` | Navigate forward/back among step rows, skipping free text |
| `step_edit_start_col()` | Finds column where step body text begins (after keyword) |
| `current_step_keyword_index()` | Identifies which keyword (Given/When/Then/And/But) is on a line |
| `replace_step_keyword_line()` | Swaps the keyword while preserving the rest of the line |
| `insert_step_below()` / `insert_step_above()` | Insert new step template lines |
| `insert_scenario_after_current()` | Insert a new scenario block |
| `delete_step()` / `delete_scenario_block()` | Remove steps or entire scenarios |
| `swap_step_with_prev()` / `swap_step_with_next()` | Reorder steps |
| `scenario_header_for_row()` / `scenario_content_rows()` | Detect scenario boundaries for folding |
| `is_feature_narrative_row()` | Detect free-text description lines |

---

## `runner.rs` — Test runner subprocess (~510 lines)

### Types

- `RunnerConfig { command, args, cwd }` — resolved from config/env/CLI
- `RunRequest { command: "run", cases: Vec<RunCase>, meta }`
- `RunEvent` — NDJSON event enum: `StartRun`, `StartCase`, `CasePassed`, `CaseFailed`, `CaseSkipped`, `Log`, `Artifact`, `EndRun`, `RunnerExit`, `RunnerError`

### Execution

- `spawn_runner(config, request)` — spawns subprocess, writes request to stdin, parses NDJSON events from stdout on a dedicated thread
- `run_cli(config, request)` — non-TUI mode for `teshi run` CLI command
- Stderr output is forwarded as `Log` events

---

## `llm.rs` — LLM streaming client (~563 lines)

### Types

- `LlmConfig { api_key, base_url, model, max_tokens, temperature }`
- `LlmRequest` / `LlmEvent` — channel message types

### Implementation

- `spawn_llm(config)` → `(Sender<LlmRequest>, Receiver<LlmEvent>)` — runs on a dedicated tokio thread
- `chat_completion()` — streams via `reqwest` SSE (Server-Sent Events)
- Parses `data: [DONE]` and `data: {"choices":[{"delta":{"content":"...","tool_calls":[...]}}]}`
- Preserves `reasoning_content` field (DeepSeek R1) for context passing
- 120-second request timeout

---

## `agent/mod.rs` + `agent/tools.rs` — AI tool system

### Tools (with JSON Schema)

| Tool | Action | Requires confirmation? |
|------|--------|------------------------|
| `get_project_info` | Returns structured summary of all features and scenarios | No |
| `highlight_mindmap_nodes` | Applies color highlights by step text pattern | No |
| `apply_mindmap_filter` | Filters the mind-map tree by step name | No |
| `get_feature_content` | Returns raw file content | No |
| `insert_scenario` | Inserts a new scenario block | **Yes** |
| `update_step` | Replaces a step line | **Yes** |

### Execution flow

`execute_tool(app, name, args, tool_call_id)` dispatches to the matching implementation. Mutation tools return `ToolResult::Queued { change }` instead of applying directly. The change is applied only after user confirmation via `accept_agent_change()`.

---

## `ui.rs` — TUI rendering (~1856 lines)

Built with **ratatui 0.29**. Uses `render_stateful_widget` for tree state.

### Layout

```
[0] Tab bar (Explore | Mind Map | AI)
[1] Horizontal separator
[2] Main panel → render_main_panel → dispatch by active tab
[3] Footer (status / agent prompt / explore footer / AI footer / key hints)
```

### Panels

| Panel | Layout | Description |
|-------|--------|-------------|
| Mind Map | 60/40 tree + AI preview | Prefix trie with category-aware coloring; AI panel shows related chat |
| Explore | 20/30/50 three columns | Features → Scenarios → Steps; inline keyword colors, run status dots |
| AI | Chat history + 3-line input | Markdown rendering, streaming partial responses with cursor |
| Editor | Full-width with highlights | Syntax highlighting, keyword alignment, cursor, selection |

### Overlays (rendered in priority order)

- Agent change prompt (bottom bar)
- External file change dialog (centered)
- Auth panel (centered, 60×20)
- Step keyword picker (positioned near current line)
- Failure detail popup (75%×70%, centered)

### Styles

| Element | Color |
|---------|-------|
| `Given` keyword | Blue |
| `When` keyword | Yellow |
| `Then` keyword | Green |
| `And`/`But` | Inherits from last major step |
| Selection background | `Rgb(64, 96, 160)` |
| Explore focused-selected bg | `Rgb(16, 64, 168)` |
| Docstring body | Italic |

---

## `highlight.rs` — Syntax highlighting (~233 lines)

- `StepHighlightState` tracks `in_doc_string` and `last_major` (keyword color inheritance for `And`/`But`)
- `highlight_line_with_state()` → `Line<Span>` with per-span colors
- Resets `last_major` on new scenario boundaries
- Keyword matching is prefix-aware (e.g., `And` does not match `Android`)

---

## `keymap.rs` — Key binding system (~733 lines)

Maps `KeyEvent` → `Action` (60+ variants) using `KeyContext` to capture mode state:

- Active tab, view stage, focus slots
- Step input active, keyword picker open, auth panel open
- Pending two-key sequences (`dd`, `yy`)

### Priority order

1. Complete `dd`/`yy` two-key sequence
2. `Ctrl+C` → global quit
3. External change prompt keys
4. Agent change prompt keys (y/n)
5. Auth panel keys (Esc)
6. Step keyword picker keys (arrows, Enter, Esc)
7. Step text input keys (Esc, Ctrl+S, Tab, etc.)
8. Explore navigation keys
9. AI focus mode keys
10. Mind map AI panel focus keys
11. Mind map tree keys
12. Default editor keys

---

## `markdown.rs` — Markdown to spans (~738 lines)

Line-level Markdown → `Vec<Line<'static>>` converter. Supports:

- Headings (`#` through `######`)
- Fenced code blocks (` ``` `)
- Blockquotes (`>`)
- Ordered/unordered lists
- Tables
- Inline formatting: `**bold**`, `*italic*`, `` `code` ``, `[text](url)`, `~~strikethrough~~`, `\$` escape

Used primarily for AI chat message rendering.

---

## `step_index.rs` — Step deduplication (~129 lines)

`StepIndex { entries: Vec<(String, Vec<usize>)> }`

- `build(project)` normalizes step text (strips keyword, lowercases) and groups by `(text, file_idx)`, storing line numbers
- `reuse_count(text)` returns how many times a normalized step appears

---

## `config/mod.rs` + `config/types.rs` — Configuration (~305 lines total)

### Resolution order

1. Hardcoded defaults: DeepSeek (`deepseek-chat`) and OpenAI (`gpt-4o`)
2. `~/.teshi/config.toml` — user overrides
3. `.teshi/config.toml` — project overrides
4. Environment variables

### Placeholder system

- `${auth:provider}` — resolves against `~/.config/teshi/auth.json`
- `${env:VAR}` — resolves from environment
- `resolve_all_placeholders()` runs after merging all layers

---

## `auth/manager.rs` — Credential storage (~203 lines)

`CredentialManager` stores `HashMap<String, CredentialEntry>` in `~/.config/teshi/auth.json`:

- Atomic writes via temp file + rename
- Unix `0o600` permissions, warns on insecure permissions
- `mask_key()` preserves first 4 + last 4 characters for display

---

## `cli/mod.rs` + `cli/auth.rs` — CLI interface

Subcommands:

- `teshi [path]` — open directory or `.feature` file
- `teshi run [--feature] [--scenario] [--runner-cmd] [--runner-cwd]` — run BDD tests
- `teshi auth login [--provider]` — interactive credential entry
- `teshi auth list` — show stored providers
- `teshi auth remove <provider>` — delete credentials
- `teshi auth status` — show config and credential paths
- `teshi auth migrate` — scan env vars and import to auth.json

---

## `gherkin_keywords.rs` — Constants (~11 lines)

```rust
STEP_KEYWORDS: ["Given", "When", "Then", "And", "But"]
HEADER_TITLE_EDIT_PREFIXES: ["Scenario Outline:", "Feature:", "Scenario:", "Examples:"]
```
