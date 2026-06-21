## Why

teshi already has Gherkin `.feature` files describing scenarios, and a step-bindings system (`.teshi/step-bindings/{feature}.json`) that maps each step to a Playwright locator. But creating those bindings today requires an engineer to manually point-and-click every element using the `bdd-locator` skill — a bottleneck that keeps feature files from becoming executable tests. The opportunity is to let an LLM agent read a Gherkin scenario, autonomously explore the web application to find each element, and generate the step bindings automatically. This closes the gap from "feature file written" to "feature file executable" without manual selector authoring.

## What Changes

- Add a **DOM element injection** system that assigns stable `[teshi-id]` attributes to all interactive elements at page load
- Add **browser interaction tools** to the LLM agent (snapshot, click, type, assert, goBack) enabling it to operate a real browser via the existing teshi-bridge WebSocket
- Add an **exploration loop** with ReAct cycle: the agent reads a Gherkin scenario, then autonomously browses the target app to fulfill each step
- Add **exploration trace persistence** to `.teshi/traces/{session_id}.jsonl`

## Capabilities

### New Capabilities

- `element-injection`: DOM traversal at page load that assigns `[teshi-id]` to every interactive element (buttons, inputs, links, selects) and provides a ref → descriptor mapping for the agent
- `browser-agent-tools`: Set of LLM-callable tools (`browser_snapshot`, `browser_click`, `browser_type`, `browser_assert`, `browser_go_back`) that operate the browser via the existing teshi-bridge WebSocket connection
- `exploration-loop`: ReAct agent loop that reads a Gherkin scenario context (feature path, step line, step text), drives the browser to fulfill it, and records a full trace with per-step DOM snapshots

### Modified Capabilities

None — no existing specs to modify.

## Impact

- **Python sidecar**: New structured snapshot command; new WS commands (click_ref, type_ref, go_back)
- **`app.rs` / `agent/`**: New browser tools in `tools.rs`; new exploration loop state machine; new trace recording
- Dependencies: May deepen existing WS protocol for richer snapshot data
