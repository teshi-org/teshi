# Browser broker migration — guarded 7.1 preparation

Task 7.1 is not complete and its OpenSpec checkbox remains unchecked. This
handoff records the independently validated launcher and endpoint preparation;
the production Chrome selector was deliberately not changed.

## Changes

- Generalized the hidden Rust broker launcher so the transport probe remains
  transport-only while the full opt-in Chrome launcher enables `p0.control` and
  all stage-5.4/5.5 observability features.
- Added the explicit development selector
  `TESHI_BROWSER_BROKER_IMPLEMENTATION=rust` and the exact paired-origin input
  `TESHI_BROWSER_BROKER_EXTENSION_ORIGINS`. Invalid, duplicate, wildcard, and
  over-limit origins fail closed; a Rust startup failure never falls back to
  Python. The legacy Python Chrome path remains available as the current
  production/default route.
- Kept Rust endpoint files credential-free and marked project compatibility
  records with `bridge=rust`. Native sidecar commands and daemon preview
  streams authenticate through the private generation-bound credential only in
  process. Rust health recovery requires the Rust selector and does not switch
  implementations.

## Validation

- `cargo check --workspace --exclude teshi-web --locked` passed.
- `cargo test --workspace --exclude teshi-web --locked -- --test-threads=1` passed
  serially, including all workspace unit/integration tests and doctests; the
  earlier parallel-only header-test failures were not reproducible serially.
- `cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings` passed.
- `cargo fmt --all -- --check`, `git diff --check`, and
  `openspec validate migrate-chrome-browser-broker-to-rust --strict` passed.
- Targeted Rust-origin, endpoint-marker, and no-credential persistence tests
  passed.

## Gate and recovery

The remaining 7.1 action is the formal default Chrome route switch. It must
wait for the exact extension pairing flow and the real-Chrome acceptance gates;
the existing migration evidence records the unavailable/closed browser
boundary. Resume from this file, `openspec/changes/migrate-chrome-browser-broker-to-rust/tasks.md`
7.1, and the current `dev` HEAD. No Python deletion, packaging, release, or
production routing change was performed.
