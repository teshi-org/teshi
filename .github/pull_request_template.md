## Summary
<!-- What changed and why. -->

## Root cause / behavior
<!-- For fixes: the failure mode before this change. For features: the new contract. -->

## Test plan
- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --exclude teshi-web --locked`
- [ ] Terminal acceptance (if TUI behavior changed): name the exact tests or shards run
- [ ] `python -m unittest discover -s scripts/tests -p 'test_ci_*.py'` (if CI or scripts changed)

## Evidence
<!-- CI run link, or the local command output. Do not merge with unchecked boxes. -->

## Known gaps
<!-- Untested platforms, deferred cases, or follow-ups. -->
