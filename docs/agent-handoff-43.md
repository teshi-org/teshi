# OpenSpec 4.3 Browser Operation Coordinator Handoff

Branch: `feat/browser-coordinator`

Baseline: `3af242e28c8084a0accdc26de0b28ce7670f17c5`

## Scope completed

- Added the internal Rust `BrowserActionCoordinator` module. It consumes the existing resolved `ExecuteLocatorCommand` and does not define a second candidate or ranking model.
- Routed `execute_browser_action` through the existing `execute_locator` command with the original `request_id`, profile identity, target, lease boundary, stream generation, snapshot ID, and page-context revision retained by the broker state machine.
- Normalized the supported typed waits into the same command: URL, visible text, page-revision change, load completion, and element state. An element-state wait reuses the action's resolved CSS/candidate recipe so the extension can re-verify the live document.
- Added structured action/wait response metadata and stable failure mapping. A completed action followed by a wait timeout is reported as `browser_wait_timeout` with `action_executed=true` and no automatic retry instruction.
- Added explicit `browser_execution_unknown` handling for a side-effecting action that was dispatched but then timed out, was cancelled, or lost its extension transport. The response includes only non-secret reconciliation metadata and reserves the request ID so a late response cannot complete a reused request.
- Added focused coordinator and broker lifecycle tests for CSS and snapshot-derived (`@e`) locators, wait validation, wait timeout, dispatched-action timeout, cancellation, late responses, and duplicate request protection.

## 4.2 integration boundary

The current state path still owns locator input parsing, candidate generation/ranking, snapshot reference resolution, frame/Shadow DOM semantics, and fixture compatibility. The coordinator is called only after that resolver returns an `ExecuteLocatorCommand`; it does not alter those algorithms or shared fixtures.

The remaining 4.2 integration point is therefore small: keep the resolver's final `ExecuteLocatorCommand` as the input to `BrowserActionCoordinator::plan`. If 4.2 changes candidate wire fields or verification metadata, adapt the coordinator's command serialization/structured-result projection without introducing another Candidate or Registry. The current compatibility DTO continues to reject caller-supplied structured `candidate` parameters until that interface is deliberately integrated.

## Validation boundary

- `cargo test -p teshi-browser-broker --locked`: 69 passed.
- `cargo clippy -p teshi-browser-broker --all-targets --all-features --locked -- -D warnings`: passed.
- `cargo test -p teshi-engine --locked`: 207 unit tests and 3 integration tests passed.
- `cargo clippy -p teshi-engine --all-targets --all-features --locked -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- `node --check extension/teshi-bridge/background.js` and `node --test extension/teshi-bridge/tests/protocol.test.mjs`: 20 passed.

No production Python routing was changed. Full workspace regression, real Chrome two-profile acceptance, and the final 4.2 replay remain outside this handoff. `docs/agent-handoff.md` is the prior 4.1 handoff and was not modified.

Local commit: this handoff is included in the local commit reported by the agent; no push or merge was performed.
