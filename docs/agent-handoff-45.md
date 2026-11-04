# OpenSpec 4.4/4.5 Browser Broker Handoff

Branch: `dev`

Baseline: `origin/dev` at `b642fbb`; the independent CI dependency fix is local commit `833f144`.

## Scope completed

- Added terminal negative coverage for ambiguous, missing, stale, hidden, disabled, timed-out, navigation-changed, and assertion-failed browser operations.
- Kept failures terminal: Rust and Python paths report `ok: false`; dispatched wait timeouts expose `action_executed: true` and do not request automatic retry.
- Fixed correlated Rust broker operation errors without a response type so the sidecar cannot wait past a terminal error packet.
- Replayed the existing `browser_profiles.feature` and `run_inspect.feature` with their existing bindings against both the Rust and Python broker paths.
- Kept Rust replay at P0 capability scope. Stage-5 screenshots are skipped when `p1.observability_artifacts` is not negotiated; Python continues to capture them.

## Real browser evidence

- Rust transport: both Features passed in real Chromium with two isolated profiles; 16/16 and 25/25 steps completed.
- Python transport: the same Features passed in real Chromium with two isolated profiles; 16/16 and 25/25 steps completed.
- Rust negative acceptance passed with `element_not_found`, `stale_element_reference`, `stale_browser_target`, `not_visible`, `element_disabled`, `assert_text_failed`, and `browser_wait_timeout`; dispatch retries were zero and timeout action count was one.
- Rust locator-context acceptance passed after navigation and returned `stale_element_reference` for the old page revision.

## Validation boundary

- `cargo fmt --all --check`, workspace check/test/clippy, and document generation passed.
- Python broker/agent/service tests, privileged setup-failure coverage, Python compilation, extension syntax check, and 21 Node protocol tests passed.
- `openspec validate migrate-chrome-browser-broker-to-rust --type change --strict --no-interactive` passed.
- `scripts/run-web-ui-smoke.sh` remains environment-blocked because `rustup` is not installed; no wasm smoke result is claimed.
- `cargo doc` retains unrelated pre-existing rustdoc warnings.

Production Chrome routing remains Python. OpenSpec stages 5–8 remain unchecked; no Python Chrome code was removed and no production routing switch was made.

No push or force-push was performed.
