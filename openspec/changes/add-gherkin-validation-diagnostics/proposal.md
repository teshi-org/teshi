## Why

Teshi's permissive Gherkin parser silently treats malformed step-looking lines such as `当用户登录` as prose, so commands such as `teshi steps unbound` can report no missing bindings even though intended steps were discarded. Teshi needs one authoritative validation and diagnostics layer before agents, CI, and execution workflows can trust parsed Feature content.

## What Changes

- Add a pure `teshi-core` Gherkin validator that preserves permissive parsing for in-progress editing while reporting syntax, structure, and suspicious unrecognized-line diagnostics with path, line, column, severity, stable code, message, and repair suggestion.
- Distinguish definite errors, such as a dialect step keyword missing its required separator, from valid Feature/Scenario description prose and lower-confidence warnings.
- Add `teshi check`, `teshi check --feature <path>`, `teshi check --all`, and machine-readable JSON output with failure exit status when error diagnostics exist.
- Require step discovery/selection and BDD execution entry points to validate their selected Feature scope first and fail closed on errors instead of returning misleading empty or partial results.
- Expose diagnostics to Desktop/Web editors without making the parser strict or preventing users from displaying and editing incomplete Feature files.
- Replace the agent-only Gherkin validation implementation with adapters over the core validator, and make Skills invoke Teshi validation instead of maintaining independent syntax rules.

## Capabilities

### New Capabilities

- `gherkin-validation-diagnostics`: Defines authoritative syntax/structure diagnostics, CLI check behavior, execution gates, editor-friendly permissive parsing, and machine-readable consumption by CI and agents.

### Modified Capabilities

- `module-boundaries`: Extends `teshi-core` ownership of Gherkin concepts to validation and diagnostics and prohibits duplicate syntax validators in agent, UI, engine, or Skill layers.

## Impact

- Core parsing/language modules and a new pure diagnostics API in `crates/teshi-core`.
- CLI command definitions and routing in `crates/teshi-tui`, plus step and run preflight behavior.
- Daemon/GPUI diagnostic transport and presentation where editors consume validation results.
- Existing `teshi-agent` `validate_feature` tooling, which becomes an adapter over core diagnostics rather than a separate rules engine.
- Teshi Skills and CI workflows, which can delegate correctness checks to `teshi check --json`.
- Tests for English and non-English dialects, malformed step separators, legal descriptions, structural failures, command exit codes, and blocked binding/execution operations.
