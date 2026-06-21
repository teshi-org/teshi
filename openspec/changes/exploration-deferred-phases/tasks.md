## 7. Distillation & Step-Binding Writing

- [ ] 7.1 Enable MutationObserver in the injection script to catch dynamically rendered elements
- [ ] 7.2 Implement multi-dimensional feature recording in the sidecar at action time
- [ ] 7.3 Build the distillation engine: deterministic scoring of candidate locators from recorded features
- [ ] 7.4 Write top-ranked locators into `.teshi/step-bindings/{feature}.json` per step_line, with `source: "agent"`
- [ ] 7.5 Read existing bindings before writing to preserve manual bindings (incremental update)
- [ ] 7.6 Add `teshi trace distill <id>` CLI command to produce a binding report from a raw trace

## 8. Self-Healing Locators & Exploration UI

- [ ] 8.1 Generate cascading fallback locators (data-testid → getByRole → relative anchor) for self-healing scripts
- [ ] 8.2 Add independent Verifier Agent for hallucination-safe assertion validation
- [ ] 8.3 Build TUI exploration panel: real-time agent state display, step timeline, action detail view, pause/resume/cancel controls
