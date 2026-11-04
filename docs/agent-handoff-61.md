# Browser broker migration — stage 6.1 handoff

Stage 6.1 is complete. OpenSpec progress is 30/47 tasks; the next task is 6.2.

## Changes

- Added Rust project/user policy loading with default-deny privileged capabilities, bounded policy files, and symlink rejection.
- Added memory-only typed capability grants with short TTLs, hash-only bearer-token storage, explicit interactive/non-interactive approval, revocation, expiry, and full OS-user, broker-generation, project, caller, extension/Profile, and capability binding.
- Added Rust broker operations to create, list, revoke, and expire grants.
- Enforced capability grants and extension-advertised optional Chrome permissions before privileged P2 dispatch. Cookie values require a separate value grant.
- Removed lease and capability bearer tokens from extension commands.

## Gates

- `cargo fmt --all -- --check`
- `cargo check --workspace --exclude teshi-web --locked`
- `cargo test --workspace --exclude teshi-web --locked`
- `cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings`
- Python browser/service unit suite: 79 passed.
- Node extension protocol and Network suites: 40 passed.
- `openspec validate migrate-chrome-browser-broker-to-rust --strict`

No production route switch, old-code deletion, or release action was performed. Raw CDP method allowlists, operation-specific gates, upload/artifact scope, and audit records remain for 6.2.
