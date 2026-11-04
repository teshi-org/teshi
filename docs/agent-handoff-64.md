# Browser broker migration — stage 6.4 handoff

Stage 6.4 is complete. OpenSpec progress is 33/47 tasks; the next task is 6.5.

Changes:

- The extension manifest test now asserts the complete explicit required permission set: `activeTab`, `alarms`, `debugger`, `storage`, `tabGroups`, and `tabs`.
- Existing extension tests continue to assert that privileged `contentSettings`, `cookies`, and `management` permissions are optional and requested only from a popup gesture, with operation-specific scope checks.
- A Rust negative integration test proves a protocol-v0 stream hello is rejected and that the retained legacy heartbeat cannot bypass the current lease and P2 capability gates.

Validation:

- `cargo fmt --all -- --check` passed.
- `cargo check --workspace --exclude teshi-web --locked` passed.
- `cargo test --workspace --exclude teshi-web --locked` passed, including 114 broker unit tests and 4 broker security integration tests.
- `cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings` passed.
- `node --test extension/teshi-bridge/tests/protocol.test.mjs` passed with 21 tests.
- The combined extension protocol and Network suite passed.
- `git diff --check` and `openspec validate migrate-chrome-browser-broker-to-rust --strict` passed.

No production routing switch, Chrome-Python deletion, or release operation was performed. The next required task is the security review in 6.5.
