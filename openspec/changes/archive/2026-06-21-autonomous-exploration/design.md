## Context

teshi manages Gherkin `.feature` files and maintains a step-bindings system (`.teshi/step-bindings/{feature}.json`) that maps each step line to a Playwright locator. Creating bindings today requires an engineer to manually use the `bdd-locator` skill — select a step in the editor, point at the element in the browser, confirm the locator.

The existing browser infrastructure (Chrome extension + Playwright sidecar via WebSocket) provides the plumbing for element discovery and interaction. What's missing is an agent that can read a feature file's scenario steps, autonomously navigate the target web application, identify each element, and write the bindings automatically.

The Phase 1 pipeline is:

```
.feature file (existing)
  → Agent reads scenario steps
  → Agent explores web app (ReAct loop)
  → Raw trace recorded to `.teshi/traces/`
```

## Goals / Non-Goals

**Goals:**
- Let an LLM agent accept a Gherkin scenario context and autonomously explore a web app to find each element
- Capture raw exploration traces for subsequent analysis
- Provide tool-based interaction with the browser via existing infrastructure

**Non-Goals:**
- Generating new Gherkin `.feature` files (they already exist as input)
- Replacing the existing manual bdd-locator workflow (both modes coexist)
- General-purpose web browsing agent (scope is BDD step binding)
- Visual regression testing or screenshot comparison
- Cross-browser support in the first pass (Chrome-only via existing CDP pipeline)

## Decisions

### Decision 1: teshi-id injection via CDP page script
**Where the element injection lives:** A JavaScript snippet injected via CDP `Page.addScriptToEvaluateOnNewDocument`, not a build-time code modification to the SUT.

**Rationale:** The SUT is not under teshi's control. Injecting via CDP at page load requires zero modification to the application being tested. The existing sidecar already uses CDP for screencast and locator execution — adding script injection reuses the same connection.

**Alternatives considered:**
- Build-time instrumentation: Only works for apps the team controls, not general BDD use
- Post-load DOM traversal from the sidecar: Adds latency and misses dynamically rendered elements before they appear
- MutationObserver in injected script: Overhead but catches dynamic content — defer to a later phase

### Decision 2: Browser tools as Rust agent extensions → WS bridge
**Where the browser tools live:** New tools registered in `agent/tools.rs` that serialize commands over the existing WebSocket to the Python sidecar.

**Rationale:** The agent loop (LLM streaming, tool dispatch, confirmation gate) is Rust-side. Adding browser tools in the same module keeps the tool execution pattern uniform. The existing WS protocol already supports `navigate`, `execute_locator`, and `get_page_snapshot` — extending it with `get_structured_snapshot`, `click_ref`, `type_ref` is the natural evolution.

**Protocol evolution:**
```
Current:  { cmd: "execute_locator", selector: "div.btn", action: "click" }
New:      { cmd: "click_ref", ref: "e15" }
          { cmd: "type_ref", ref: "e22", text: "admin" }
          { cmd: "get_structured_snapshot" }  // returns AOM-tree style snapshot
```

### Decision 3: Gherkin-scenario-driven exploration loop
**How the loop is triggered:** The user selects a scenario in the editor (or highlights specific steps), then invokes "explore". The agent receives the feature path, step lines, and step texts as context, and enters the ReAct loop with the goal of completing each step in sequence.

**Rationale:** The scenario steps define the mission. Each step text tells the agent what to look for ("I click the Save button" → find a button with text "Save"). Without this context, the agent would be exploring aimlessly. The existing `step_index.rs` and `bdd_nav.rs` already know how to enumerate scenarios and steps — the exploration loop hooks into that.

**State machine:**
```
Idle → ExploreActive (agent reads scenario, runs ReAct loop)
         ├── Paused (user pressed pause, waiting for resume)
         ├── StepComplete (current step bound, moving to next step)
         ├── AllComplete (all steps in scenario have bindings)
         └── Failed (step limit / boundary violation / error)
```

**Step-level flow:**
```
For each step in scenario:
  1. Agent gets step text (e.g. "I click the 'Save' button")
  2. Agent calls browser_snapshot → sees page state
  3. Agent decides action → browser_click(ref) 
  4. If action succeeds → mark step as bound, move to next step
  5. If action fails → retry or alternate approach
  6. After all steps → trace saved
```

### Decision 4: Phase 1 scope
**Scope:** teshi-id injection, browser agent tools, Gherkin-scenario-driven exploration loop (with URL sandbox and step limits), and basic trace recording.

**Deferred phases** (distillation, self-healing, binding writing, exploration UI) have been moved to the separate change `exploration-deferred-phases`.

## Risks / Trade-offs

| Risk | Impact | Mitigation |
|------|--------|------------|
| **CDP script injection not supported in all browsers** | Chrome-only limitation | Already the case — embedded mode uses Chromium; Chrome extension is Chrome-only. Accept as a design constraint. |
| **Step text too vague for element discovery** | Agent cannot map "I do the thing" to any element | Fall back to human-in-the-loop: agent pauses and asks user for clarification |
| **WebSocket protocol becomes complex** | Bridge maintenance burden | Keep ref-based commands as thin wrappers over existing CDP calls |
| **Maximum 15 steps too restrictive for long scenarios** | Valid tests fail to complete | Make configurable per scenario; 15 is a safe default that catches infinite loops |

## Open Questions

1. **How does the user select a scenario for exploration?** From the editor (cursor on a step → "explore from here")? Or from the explore tab (pick a scenario from the feature tree)?
2. **What is the success signal per step?** The agent calls `browser_assert` explicitly, or the loop assumes click/type success means step done?
3. **How are step bindings written incrementally?** Auto-write on each step completion, or batch-write all at once after the full scenario succeeds?
4. **Should the agent be allowed to skip steps?** If a step is "And I am logged in" and the user is already logged in, skip vs. verify?
