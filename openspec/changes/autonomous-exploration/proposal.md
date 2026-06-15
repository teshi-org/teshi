## Why

teshi already has Gherkin `.feature` files describing scenarios, and a step-bindings system (`.teshi/step-bindings/{feature}.json`) that maps each step to a Playwright locator. But creating those bindings today requires an engineer to manually point-and-click every element using the `bdd-locator` skill — a bottleneck that keeps feature files from becoming executable tests. The opportunity is to let an LLM agent read a Gherkin scenario, autonomously explore the web application to find each element, and generate the step bindings automatically. This closes the gap from "feature file written" to "feature file executable" without manual selector authoring.

## What Changes

- Add a **DOM element injection** system that assigns stable `[teshi-id]` attributes to all interactive elements at page load
- Add **browser interaction tools** to the LLM agent (snapshot, click, type, assert, goBack) enabling it to operate a real browser via the existing teshi-bridge WebSocket
- Add an **exploration loop** with ReAct cycle: the agent reads a Gherkin scenario, then autonomously browses the target app to fulfill each step, recording locator data along the way
- Add a **distillation engine** that transforms raw exploration traces (`Click(id=15)`) into stable Playwright locators (`getByRole('button', {name: '登录'})`)
- Add **multi-level fallback locator scoring** producing self-healing selectors (testid → role → relative positioning)
- Add **step-binding generation** that writes distilled locators into the existing `.teshi/step-bindings/{feature}.json` format, keyed by `step_line`
- Add an **exploration UI panel** in the TUI showing real-time agent browser state, trace replay, and manual override controls

## Capabilities

### New Capabilities

- `element-injection`: DOM traversal at page load that assigns `[teshi-id]` to every interactive element (buttons, inputs, links, selects) and provides a ref → descriptor mapping for the agent
- `browser-agent-tools`: Set of LLM-callable tools (`browser_snapshot`, `browser_click`, `browser_type`, `browser_assert`, `browser_go_back`) that operate the browser via the existing teshi-bridge WebSocket connection
- `exploration-loop`: ReAct agent loop that reads a Gherkin scenario context (feature path, step line, step text), drives the browser to fulfill it, and records a full trace with per-step DOM snapshots
- `locator-distillation`: Engine that reads raw exploration traces plus per-element multi-dimensional features (DOM path, accessibility attributes, text context) and produces ranked, stable Playwright locators
- `self-healing-script`: Locator scoring system producing cascading fallback selectors (data-testid → getByRole → relative text anchor)
- `step-binding-generation`: Writes distilled locators into the existing `.teshi/step-bindings/{feature}.json` format, mapping each explored action to the corresponding `step_line` from the feature file
- `exploration-ui`: New TUI panel showing live agent browser preview, step trace with screenshots, manual pause/override/resume controls

### Modified Capabilities

None — no existing specs to modify.

## Impact

- **`crates/teshi-runtime`**: New JS injection module for `[teshi-id]` DOM traversal; new ref mapping management; extended `StepBinding` writing path
- **Python sidecar**: New structured snapshot command; new feature capture alongside existing locator execution
- **`app.rs` / `agent/`**: New browser tools in `tools.rs`; new exploration loop state machine; new distillation + binding wiring
- **`ui.rs`**: New exploration panel layout and rendering
- Dependencies: May deepen existing WS protocol for richer snapshot data
