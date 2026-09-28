# Browser broker migration — stage 6.5 handoff

Stage 6.5 is complete. OpenSpec progress is 34/47 tasks; the next task is 7.1.

Changes:

- Added `docs/security-review-65.md` covering discovery, loopback/Host/Origin/token permissions, sensitive logging and audit projections, project/caller/Profile/generation scoping, legacy protocol-v0 behavior, and server-side dispatch gates.
- Added defense-in-depth validation in the state owner so direct `BrokerEvent::Operation` callers cannot bypass the supported-operation or schema-version checks that the WebSocket transport performs.
- Added a negative integration test for direct state injection of an unknown operation and incompatible schema.

Validation:

- `cargo test --workspace --exclude teshi-web --locked` passed, including 114 broker unit tests and 5 broker security integration tests.
- `cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings` passed.
- `cargo check --workspace --exclude teshi-web --locked` and `cargo fmt --all -- --check` passed.
- `node --test extension/teshi-bridge/tests/protocol.test.mjs extension/teshi-bridge/tests/network-capture.test.mjs` passed with 40 tests.
- `git diff --check` and `openspec validate migrate-chrome-browser-broker-to-rust --strict` passed.

No production routing switch, Chrome-Python deletion, packaging, or release operation was performed. Stage 7.1 must first switch only Chrome startup to the Rust launcher while preserving Embedded and WinApp Python paths.
