## 1. DOM Element Injection (teshi-id)

- [x] 1.1 Create the JS injection script that traverses the DOM and assigns `teshi-id` attributes to all interactive elements (buttons, inputs, links, selects, textareas, elements with tabindex/role)
- [x] 1.2 Implement CDP `Page.addScriptToEvaluateOnNewDocument` injection via the Python sidecar's existing CDP connection
- [x] 1.3 Build the ref → element descriptor mapping table, keyed by teshi-id, storing role, accessible name, tag, and element type
- [x] 1.4 Wire the injection so it runs on every page navigation / new document
- [x] 1.5 Add a `get_structured_snapshot` command to the sidecar's WS protocol that returns the current ref mapping as a structured list (LLM-friendly AOM-tree style)

## 2. Browser Agent Tools (Rust-side)

- [x] 2.1 Add `browser_snapshot` tool to `agent/tools.rs` — calls `get_structured_snapshot` over WS, returns parsed element list to the LLM
- [x] 2.2 Add `browser_click(ref)` tool — sends `click_ref` command over WS, returns success/error
- [x] 2.3 Add `browser_type(ref, text)` tool — sends `type_ref` command over WS, returns success/error
- [x] 2.4 Add `browser_assert(condition, type)` tool — checks text visibility or URL match on current page state
- [x] 2.5 Add `browser_go_back` tool — sends navigation back command over WS
- [x] 2.6 Register all new tools in the LLM tool schema with JSON Schema definitions so the LLM discovers them

## 3. Gherkin-Scenario-Driven Exploration Loop

- [x] 3.1 Add a mechanism to extract scenario context from the active buffer: feature path, step lines, step texts, and keywords (Given/When/Then)
- [x] 3.2 Define `AgentMode` enum with `Idle` and `Explore` variants in `agent/mod.rs`; in Explore mode, the available tool set expands to include browser tools
- [x] 3.3 Implement the step-level exploration flow: for each step in the scenario, the agent observes → decides → acts, then moves to the next step
- [x] 3.4 Implement the exploration trace buffer: ordered list of actions with step_line, timestamp, ref, action type, arguments, and resulting snapshot
- [x] 3.5 Add step counter with configurable max steps (default 15); terminate and mark trace incomplete on exceed
- [x] 3.6 Implement URL whitelist sandbox — check page URL against allowed patterns after each navigation; on violation trigger goBack and log boundary violation
- [x] 3.7 Add `reset_environment` tool that restores application state to a clean baseline before exploration starts
- [x] 3.8 Wire the exploration mode into `poll_llm_events()` so the agent loop switches between chat mode and explore mode
- [x] 3.9 Handle loop termination conditions: all steps bound, step limit exceeded, URL boundary violation, unrecoverable error, user cancel

## 4. Sidecar Protocol Extension

- [x] 4.1 Add `click_ref{ref}` command handler in the Python sidecar — resolves ref → CDP node, executes click via CDP `Input.dispatchMouseEvent`
- [x] 4.2 Add `type_ref{ref, text}` command handler — resolves ref → CDP node, focuses element, dispatches `Input.insertText`
- [x] 4.3 Add `get_structured_snapshot{}` command handler — traverses CDP accessibility tree and returns structured element list with ref, role, name, type
- [x] 4.4 Add `go_back{}` command handler — calls CDP to navigate back one page
- [x] 4.5 Wire all new commands into the existing WS dispatch loop alongside existing locator commands

## 5. Exploration Trace Persistence

- [x] 5.1 Define trace JSONL schema: one JSON object per action with step_line, action type, ref, arguments, timestamp, URL, and structured snapshot
- [x] 5.2 Write trace buffer to `.teshi/traces/{session_id}.jsonl` on completion or termination
- [x] 5.3 Add CLI command `teshi trace list` to show available traces and `teshi trace show <id>` to view a trace

## 6. Integration & Wiring

- [x] 6.1 Connect exploration start to a user action (e.g., select a scenario in the editor → run "explore scenario")
- [x] 6.2 Ensure exploration mode properly pauses/resumes existing LLM streaming when switching between chat and explore
- [ ] 6.3 Test the full flow end-to-end: select scenario → snapshot → agent decides → click/type → assert → next step → all done → trace saved
- [ ] 6.4 Add `cargo test` tests for tool JSON Schema serialization, state machine transitions, URL sandbox logic, and step counter

---

## Deferred — Phase 2 (Distillation & Step-Binding Writing)

> These tasks are designed but NOT scoped for the current implementation phase.

- [ ] 7.1 Enable MutationObserver in the injection script to catch dynamically rendered elements
- [ ] 7.2 Implement multi-dimensional feature recording in the sidecar at action time
- [ ] 7.3 Build the distillation engine: deterministic scoring of candidate locators from recorded features
- [ ] 7.4 Write top-ranked locators into `.teshi/step-bindings/{feature}.json` per step_line, with `source: "agent"`
- [ ] 7.5 Read existing bindings before writing to preserve manual bindings (incremental update)
- [ ] 7.6 Add `teshi trace distill <id>` CLI command to produce a binding report from a raw trace

## Deferred — Phase 3 (Self-Healing Locators & Exploration UI)

> These tasks are designed but NOT scoped for the current implementation phase.

- [ ] 8.1 Generate cascading fallback locators (data-testid → getByRole → relative anchor) for self-healing scripts
- [ ] 8.2 Add independent Verifier Agent for hallucination-safe assertion validation
- [ ] 8.3 Build TUI exploration panel: real-time agent state display, step timeline, action detail view, pause/resume/cancel controls
