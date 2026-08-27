## Why

Teshi's new multi-profile broker can safely discover, lease, inspect, and mutate explicitly targeted Chromium profiles, but its CLI remains too narrow for reliable end-to-end browser work: actions accept only CSS selectors, Chrome clicks are synthetic DOM clicks, the broker must normally be started through Desktop, and common tab, wait, artifact, and debugging workflows are missing. Closing those gaps in controlled phases will make Teshi useful as a standalone browser-control CLI without discarding its stronger lease, locator-verification, BDD, and least-privilege boundaries.

## What Changes

- **P0 - dependable control loop:** allow the CLI to start or reconnect the Chrome broker without Desktop; expose profile lookup and unique label management; add snapshot-scoped element references; execute structured locator candidates or references without degrading them to CSS; provide CDP-backed mouse input; add post-action waits; manage tabs; and align supported actions across CLI, broker, extension, replay, and documentation.
- **P1 - observability and artifacts:** add viewport, full-page, and element screenshots, PDF output, bounded console and network capture, operation before/after summaries, file upload, and explicit window/tab-group organization, all scoped to the selected target and lease.
- **P2 - privileged escape hatches:** add separately gated arbitrary JavaScript, raw CDP, cookie access, and browser content-setting or extension-management operations with explicit capability grants, audit records, redaction, size limits, and fail-closed defaults.
- Preserve the canonical `extension_instance_id + window_id + tab_id` target, exclusive profile lease, request correlation, page-context revision, same-host execution, and ambiguous-target failure behavior across every phase.
- Keep locator acquisition observational by default. A locator result is executed only through a separate, explicitly requested control operation.
- Maintain compatibility for legacy implicit commands only when exactly one eligible target exists; new multi-profile and privileged operations require explicit targeting.

## Capabilities

### New Capabilities

- `browser-control-primitives`: P0 snapshot references, structured locator execution, real pointer input, post-action waits, tab/window targeting, and consistent action semantics.
- `browser-observability-artifacts`: P1 screenshots, PDFs, console/network capture, operation diffs, uploads, and bounded artifact handling.
- `privileged-browser-access`: P2 capability grants and safety contracts for arbitrary JavaScript, raw CDP, cookies, content settings, and extension management.

### Modified Capabilities

- `browser-extension-connection`: Add CLI-owned broker bootstrap/reconnect behavior and extension support for the phased control, observability, and privileged protocols.
- `multi-browser-session-broker`: Extend explicit target, lease, request-correlation, isolation, and ambiguity guarantees to element references, tab/window lifecycle, artifacts, diagnostics, and privileged operations.
- `external-agent-testing-interface`: Expand the machine-readable CLI operation surface while preserving typed errors, explicit ownership, same-host execution, and narrow MCP exposure.
- `playwright-locator-acquisition`: Make verified structured candidates executable by a separate authorized control operation without converting semantic locators to CSS.
- `browser-agent-tools`: Unify legacy browser refs and actions with the canonical multi-profile target, lease, page revision, and shared operation contracts.
- `agent-testing-distribution`: Package and document the expanded CLI workflow, compatibility declarations, extension assets, capability policy, and phased setup.

## Impact

- Affects `crates/teshi-engine` browser operation and capability models, `crates/teshi-tui` browser CLI and MCP adapter, `resources/browser_agent_broker.py`, `resources/browser_service.py`, and `extension/teshi-bridge`.
- Affects `.teshi/cdp-endpoint.json`, local capability-policy and audit storage, snapshot/reference caches, artifact directories, step-binding execution, and browser diagnostics.
- Adds Chrome debugger-domain usage for pointer input, screenshots, PDF, console/network capture, and explicitly authorized raw CDP; broader extension permissions MUST be requested only when required by the enabled P2 capability.
- Affects release archives/MSI, the `teshi-browser-testing` package, compatibility metadata, browser-mode documentation, and smoke/real-browser validation.
- P0, P1, and P2 are implementation gates in that order; P2 MUST NOT be enabled merely because P0 or P1 is installed.
