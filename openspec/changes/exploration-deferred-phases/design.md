## Context

This change continues the autonomous exploration work. Phase 1 established the
core exploration loop (teshi-id injection, browser agent tools, Gherkin-driven
ReAct loop, trace persistence). This change builds on top of that foundation
to turn raw traces into durable, production-ready step bindings.

## Decisions

### Decision 1: Multi-dimensional feature recording in the Python sidecar
**Where feature capture happens:** The Python sidecar records per-element features
at the time of each `click_ref`/`type_ref` command, using CDP's `DOM.getDocument`
and `Accessibility.getFullAXTree` to collect DOM path, accessibility attributes,
text content, and CSS selector candidates.

**Rationale:** The CDP connection is in the Python process. Sending raw DOM back
to Rust for feature extraction would double the data volume. Recording features
at action time (rather than batch after exploration) keeps the trace compact and
avoids state drift.

**Data structure per recorded action:**
```python
{
  "step_line": 12,
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

### Decision 2: Distillation engine as a Rust-side pipeline step
**Where locator distillation runs:** A new `distill` function in the agent module
that takes the raw trace and produces ranked locator candidates using deterministic
scoring rules (not LLM-dependent for the core ranking).

**Rationale:** LLM calls for distillation would be slow and expensive per trace.
The stability scoring rules are well-defined heuristics (prioritize data-testid,
penalize dynamic class hashes, prefer semantic role+name over structural XPath).
Deterministic scoring ensures reproducibility. The LLM may be used later to verify
unclear cases.

**Scoring algorithm (simplified):**
```
data-testid present  → score 100  (exact match, stable)
id/name static       → score 80   (stable if not auto-generated)
getByRole + name     → score 70   (semantic, readable)
relative text anchor → score 50   (context-dependent)
combined attributes  → score 30   (fragile on reorder)
class-based XPath    → score 10   (brittle, hash-dependent)
```

### Decision 3: Step-binding generation writes existing format
**How bindings are persisted:** After distillation, the top-ranked locator per step
is written as a `StepBinding` entry into the existing `.teshi/step-bindings/{feature}.json`
file, using the same schema as manual recordings but with `source: "agent"`.

**Rationale:** The existing replay system already reads this format. Writing
agent-generated bindings in the same format means they work immediately with
`teshi browser replay`, the web UI's step-status badges, and any other tooling
built on step bindings. No new storage format needed.

### Decision 4: Self-healing with cascading fallback
**How scripts become resilient:** Each element interaction is backed by a chain of
3+ candidate locators tried in priority order.

**Rationale:** Applications change over time. A locator that works today may break
after a UI refactor. Cascading fallbacks let scripts self-heal without human
intervention. The priority order (data-testid → getByRole → relative anchor)
reflects typical stability profiles.

### Decision 5: Independent Verifier Agent
**Where assertion validation lives:** A separate lightweight agent that runs after
each step, receives the step text and the current page state, and independently
confirms whether the assertion condition is met.

**Rationale:** LLMs hallucinate. An agent that "thinks" it clicked the right button
when it actually clicked the wrong one is a real risk. A separate verifier with a
different prompt and temperature can catch these cases.

### Decision 6: TUI exploration panel
**Where the panel appears:** A new tab/layout in the existing TUI showing the
agent's live view of the browser, a step timeline with screenshots, and
pause/resume/cancel controls.

**Rationale:** Exploration is opaque without visual feedback. Users need to see
what the agent is doing to trust its output. The existing TUI framework in `ui.rs`
already supports tab-based layouts — adding an explore tab follows the existing
pattern.
