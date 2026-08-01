# Architecture

## High-level design

teshi is a single-binary terminal application built with **ratatui** + **crossterm**. It parses Gherkin `.feature` files into an AST, maintains an editable rope buffer per file, indexes steps into a prefix trie for mind-map navigation, and optionally spawns an LLM subprocess for AI assistance and a test runner subprocess for BDD execution.

## Data flow

```
CLI args (clap)
  │
  ▼
gherkin::parse_project() ──► BddProject (AST)
  │                              │
  │                    ┌─────────┼──────────┐
  │                    ▼         ▼          ▼
  │              StepIndex   EditorBuffer  MindMapIndex
  │              (deduped    (rope per     (prefix trie)
  │               steps)      file)
  │
  ▼
App::new() ──► main event loop
                  │
     ┌────────────┼────────────┬──────────────┐
     ▼            ▼            ▼              ▼
  poll runner   poll LLM    poll disk      read input
  events        events      changes        (50ms tick)
     │            │            │              │
     └────────────┼────────────┴──────────────┘
                  ▼
           handle_action()
                  │
                  ▼
           ui::render()  ← ratatui frame
```

## Event loop

The main loop (in `main.rs`) runs on a 50ms tick and processes:

1. **Runner events** — non-blocking `try_recv` from the runner channel; dispatches to `apply_run_event()`
2. **LLM events** — streaming chunks, tool-call requests, completions, and errors; drives the agent loop
3. **File change detection** — polls `FileStamp` (mtime + size) every 250ms; auto-reloads or prompts
4. **Status message expiry** — auto-clears after 3 seconds
5. **Render** — `ui::render(frame, &mut app)` draws the full TUI
6. **Input** — reads keyboard events, maps them to `Action` via `keymap.rs`, calls `app.handle_action(action)`

## Module dependency graph

```
main.rs
 ├── cli/          (clap subcommands: auth, run)
 │    ├── auth/    (credential manager)
 │    └── config/  (layered TOML + env + placeholder resolution)
 │
 └── app.rs        (core orchestrator — ~4300 lines)
      ├── gherkin.rs         (hand-written .feature parser → BddProject AST)
      ├── mindmap.rs         (prefix trie → tui-tree-widget items)
      ├── step_index.rs      (normalized step deduplication)
      ├── editor_buffer.rs   (ropey::Rope wrapper, undo/redo snapshots)
      ├── bdd_nav.rs         (structured BDD navigation & editing operations)
      ├── runner.rs          (NDJSON subprocess protocol)
      ├── llm.rs             (SSE streaming HTTP client)
      ├── agent/             (function-calling tool system)
      │    └── tools.rs      (6 LLM tools with JSON Schema)
      ├── highlight.rs       (Gherkin syntax highlighting)
      ├── markdown.rs        (Markdown → ratatui Spans)
      ├── keymap.rs          (KeyEvent → Action dispatch, 60+ actions)
      ├── gherkin_keywords.rs (shared keyword constants)
      └── config/            (config types and loader)
```

## Buffer model

The application maintains **two buffer concepts**:

- **`buffers: Vec<EditorBuffer>`** — one persistent `ropey::Rope` per `.feature` file, preserving full undo/redo stacks
- **`buffer: EditorBuffer`** — the active editable view, cloned from `buffers[idx]` when switching files

When navigating between files (`switch_to_buffer`), the current active buffer is snapshotted back into `buffers[idx]` (preserving cursor, scroll, undo), and the new file's buffer is cloned into the active view. This avoids data loss when switching contexts.

## View stages (Mind Map tab)

The mind-map tab has three stages that progressively reveal more detail:

| Stage | Layout | Trigger |
|-------|--------|---------|
| `TreeOnly` | Full-width trie tree | Default |
| `TreeAndEditor` | 60% tree + 40% editor preview | `Enter` on a tree node |
| `EditorAndPanel` | Full editor + run panel | Open from explore or tree |

Transitions: `Enter` deepens, `Esc` backs out, `Ctrl+\` toggles the AI panel within the mind-map view.

## Agent loop

The AI integration uses a **human-in-the-loop** pattern for mutations:

```
User message → LLM (SSE stream)
  │
  ▼
LLM responds with tool calls
  │
  ├── Read-only tools (get_project_info, highlight, filter, get_feature_content)
  │     → Execute immediately → feed result back to LLM → continue loop
  │
  └── Mutation tools (insert_scenario, update_step)
        → Queue AgentPendingChange → pause loop → show diff to user
        → User presses y (accept) or n (reject)
        → Feed result to LLM → continue loop (max 5 iterations)
```

This ensures the LLM cannot modify files without explicit user consent.

## Runner protocol

The test runner is an **external subprocess** communicating via NDJSON:

**Request** (one line on stdin):
```json
{"command":"run","cases":[{"id":"f0:s1","feature_path":"login.feature","scenario":"Successful login","line_number":12}],"meta":{}}
```

**Events** (one per line on stdout):
```json
{"type":"start_run","total":3}
{"type":"start_case","case_id":"f0:s1","name":"Successful login"}
{"type":"case_passed","case_id":"f0:s1","duration_ms":1234}
{"type":"case_failed","case_id":"f0:s2","duration_ms":567,"error":{"message":"Assertion failed","stack":"..."}}
{"type":"case_skipped","case_id":"f0:s3","reason":"Not implemented"}
{"type":"end_run","passed":1,"failed":1,"skipped":1}
```

## Configuration layering

Configuration is resolved from five sources in priority order (highest wins):

1. CLI flags (`--runner-cmd`, `--runner-cwd`)
2. Environment variables (`TESHI_RUNNER_CMD`, `TESHI_LLM_API_KEY`, etc.)
3. Project-level `.teshi/config.toml`
4. User-level `~/.teshi/config.toml`
5. Hardcoded defaults (DeepSeek and OpenAI provider definitions)

API keys support `${auth:provider}` placeholders that resolve against `~/.config/teshi/auth.json` (stored with `0600` permissions), keeping secrets out of project config files.
