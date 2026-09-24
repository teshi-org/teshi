## Why

The Gherkin MindMap currently rewinds from a `Then` step to an earlier parent when another `When` appears, so one scenario's ordered steps are displayed as separate branches. This hides the scenario's actual execution order and makes complex flows misleading to inspect.

## What Changes

- Build every scenario path from its steps in source order, including transitions such as `When → Then → When → Then`.
- Continue sharing identical prefixes between scenarios while branching only where their ordered step sequences differ.
- Add a sufficiently complex layout fixture covering Background steps, repeated keyword groups, conjunctions, shared prefixes, divergent scenarios, and Rule-nested scenarios.
- Add structural assertions that verify parent-child topology rather than only checking that MindMap nodes exist.

## Capabilities

### New Capabilities

- `gherkin-mindmap-sequence`: Defines how ordered Gherkin scenario steps are represented and prefix-shared in the MindMap.

### Modified Capabilities

None.

## Impact

- Affects the pure MindMap index builder and its unit tests in `teshi-core`.
- Changes the displayed topology for scenarios containing multiple result/action groups or consecutive result steps.
- Does not change the Gherkin parser, source locations, TUI rendering APIs, or external dependencies.
