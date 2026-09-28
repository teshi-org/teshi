# Browser broker migration — stage 6.3 handoff

Stage 6.3 is complete. OpenSpec progress is 32/47 tasks; the next task is 6.4.

## Changes

- Added `crates/teshi-browser-broker/tests/security_negative.rs` with public-boundary tests for hostile Origins and stale tokens, unknown operations and strict malformed targets, and configured oversized extension WebSocket messages.
- Kept the existing broker negative coverage for managed-artifact traversal/symlink escape, grant revocation/expiry, malformed TSH1 frames, and disconnect/late-response/reuse races; the broker suite exercises those paths together with the new integration suite.

## Gates

- `cargo fmt --all -- --check`
- `cargo check --workspace --exclude teshi-web --locked`
- `cargo test -p teshi-browser-broker --locked` — 114 unit tests and 3 negative integration tests passed.
- `cargo test --workspace --exclude teshi-web --locked` — exit code 0.
- `cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings`
- `git diff --check`

No production route switch, old-code deletion, or release action was performed. The next task is verification of explicit extension permissions and protocol-v0 authorization behavior in 6.4.
