# Exploration — Deferred Phases (Distillation, Self-Healing & UI)

## Why

These phases were designed as part of the `autonomous-exploration` change but
deferred to focus on getting the core exploration loop working first. They
extend the raw exploration traces into actionable step bindings with stable,
self-healing locators and provide real-time visibility into the agent's
decision process.

## What Changes

- **Multi-dimensional feature recording** captures per-element attributes (DOM path,
  accessibility data, CSS candidates) at action time in the Python sidecar
- **Distillation engine** scores candidate locators by stability and produces
  ranked recommendations (data-testid → role+name → relative anchor)
- **Step-binding generation** writes agent-discovered locators into the existing
  `.teshi/step-bindings/{feature}.json` format with `source: "agent"`
- **Self-healing scripts** with cascading fallback chains (3+ levels per element)
- **Independent Verifier Agent** for hallucination-safe assertion validation
- **Exploration UI panel** in the TUI showing real-time agent state, trace replay,
  and manual override controls

## Capabilities

### New Capabilities

- `locator-distillation`: Engine that reads raw exploration traces plus per-element
  multi-dimensional features and produces ranked, stable Playwright locators
- `self-healing-script`: Locator scoring system producing cascading fallback selectors
- `step-binding-generation`: Writes distilled locators into `.teshi/step-bindings/`
  format, mapping explored actions to `step_line`
- `exploration-ui`: TUI panel with live agent browser preview, step trace replay,
  and pause/override/resume controls

### Modified Capabilities

None.
