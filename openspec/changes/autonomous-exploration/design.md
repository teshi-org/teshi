## Context

teshi manages Gherkin `.feature` files and maintains a step-bindings system (`.teshi/step-bindings/{feature}.json`) that maps each step line to a Playwright locator. Creating bindings today requires an engineer to manually use the `bdd-locator` skill — select a step in the editor, point at the element in the browser, confirm the locator.

The existing browser infrastructure (Chrome extension + Playwright sidecar via WebSocket) provides the plumbing for element discovery and interaction. What's missing is an agent that can read a feature file's scenario steps, autonomously navigate the target web application, identify each element, and write the bindings automatically.

The corrected pipeline is:

```
.feature file (existing)
  → Agent reads scenario steps
  → Agent explores web app (ReAct loop)
  → Distillation: raw trace → stable locators
  → Write .teshi/step-bindings/{feature}.json
```

## Goals / Non-Goals

**Goals:**
- Let an LLM agent accept a Gherkin scenario context and autonomously explore a web app to find each element
- Capture raw exploration traces with per-element multi-dimensional feature data for subsequent distillation
- Distill raw traces into ranked, stable Playwright locators using deterministic scoring
- Write distilled locators into the existing `.teshi/step-bindings/{feature}.json` format
- Provide cascading fallback locators (data-testid → getByRole → relative anchor) for self-healing
- Provide a TUI panel for monitoring, replaying, and controlling exploration sessions

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
  6. After all steps → distillation → write bindings
```

### Decision 4: Multi-dimensional feature recording in the Python sidecar
**Where feature capture happens:** The Python sidecar records per-element features at the time of each `click_ref`/`type_ref` command, using CDP's `DOM.getDocument` and `Accessibility.getFullAXTree` to collect DOM path, accessibility attributes, text content, and CSS selector candidates.

**Rationale:** The CDP connection is in the Python process. Sending raw DOM back to Rust for feature extraction would double the data volume. Recording features at action time (rather than batch after exploration) keeps the trace compact and avoids state drift.

**Data structure per recorded action:**
```python
{
  "step_line": 12,          # links back to the Gherkin step
  "action": "click",
  "ref": "e15",
  "timestamp_ms": 123456,
  "url": "https://example.com/login",
  "features": {
    "tag": "button",
    "role": "button",
    "name": "Sign In",
    "text": "Sign In",
    "xpath": "//*[@id='login-form']/button",
    "css_candidates": ["#login-form > button", "form button.primary"],
    "data_testid": null,
    "attributes": {"id": "signin-btn", "class": "btn primary"},
    "has_dynamic_class": False,
    "bounding_box": {"x": 100, "y": 200, "width": 120, "height": 40}
  }
}
```

### Decision 5: Distillation engine as a Rust-side pipeline step
**Where locator distillation runs:** A new `distill` function in the agent module that takes the raw trace and produces ranked locator candidates using deterministic scoring rules (not LLM-dependent for the core ranking).

**Rationale:** LLM calls for distillation would be slow and expensive per trace. The stability scoring rules are well-defined heuristics (prioritize data-testid, penalize dynamic class hashes, prefer semantic role+name over structural XPath). Deterministic scoring ensures reproducibility. The LLM may be used later to verify unclear cases.

**Scoring algorithm (simplified):**
```
data-testid present  → score 100  (exact match, stable)
id/name static       → score 80   (stable if not auto-generated)
getByRole + name     → score 70   (semantic, readable)
relative text anchor → score 50   (context-dependent)
combined attributes  → score 30   (fragile on reorder)
class-based XPath    → score 10   (brittle, hash-dependent)
```

### Decision 6: Step-binding generation writes existing format
**How bindings are persisted:** After distillation, the top-ranked locator per step is written as a `StepBinding` entry into the existing `.teshi/step-bindings/{feature}.json` file, using the same schema as manual recordings but with `source: "agent"`.

**Rationale:** The existing replay system already reads this format. Writing agent-generated bindings in the same format means they work immediately with `teshi browser replay`, the web UI's step-status badges, and any other tooling built on step bindings. No new storage format needed.

### Decision 7: Task scoping — Phase 1 only for initial implementation
**What we build first:** The first implementation phase covers teshi-id injection, browser agent tools, Gherkin-scenario-driven exploration loop (with URL sandbox and step limits), and basic trace recording. Distillation, self-healing locator scoring, and step-binding writing are designed now but deferred.

**Rationale:** Each phase builds on the previous. The exploration loop is worthless without browser tools, which are worthless without element injection. Getting a working end-to-end demo (agent reads a feature scenario → explores → completes steps) validates the core architecture before investing in the distillation and binding pipeline.

## Risks / Trade-offs

| Risk | Impact | Mitigation |
|------|--------|------------|
| **CDP script injection not supported in all browsers** | Chrome-only limitation | Already the case — embedded mode uses Chromium; Chrome extension is Chrome-only. Accept as a design constraint. |
| **LLM hallucinates page state** | Agent asserts success when page shows error | Independent Verifier Agent (deferred to later phase); for Phase 1, require screenshot + DOM snippet in trace |
| **Step text too vague for element discovery** | Agent cannot map "I do the thing" to any element | Fall back to human-in-the-loop: agent pauses and asks user for clarification |
| **Dynamic single-page apps break teshi-id injection** | Elements rendered after page load won't have IDs | MutationObserver in injected script (deferred); for Phase 1, require page reload or explicit snapshot refresh |
| **WebSocket protocol becomes complex** | Bridge maintenance burden | Keep ref-based commands as thin wrappers over existing CDP calls; no bridge-side logic beyond feature capture |
| **Maximum 15 steps too restrictive for long scenarios** | Valid tests fail to complete | Make configurable per scenario; 15 is a safe default that catches infinite loops |

## Open Questions

1. **How does the user select a scenario for exploration?** From the editor (cursor on a step → "explore from here")? Or from the explore tab (pick a scenario from the feature tree)?
2. **What is the success signal per step?** The agent calls `browser_assert` explicitly, or the loop assumes click/type success means step done?
3. **How are step bindings written incrementally?** Auto-write on each step completion, or batch-write all at once after the full scenario succeeds?
4. **Should the agent be allowed to skip steps?** If a step is "And I am logged in" and the user is already logged in, skip vs. verify?
