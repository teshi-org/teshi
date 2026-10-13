# Chrome Rust migration: stage 8.6 integration documentation

Validated on Windows on 2026-10-13 against base revision `3f38d8a` plus
the stage 8.6 working-tree changes. Task 8.6 is complete; progress is 44/47.
The subsequent [final local acceptance](agent-handoff-87-88.md) supersedes the
remaining-gates snapshot below and records the user-approved macOS deferral.

## Changes and contracts

- Browser mode and extension setup documentation now describe the Rust default,
  the public durable pairing commands, exact per-user origin approval, live
  revocation, project-neutral public discovery, and shared broker lifetime.
- The packaged locator skill and compatibility reference document Python-free
  Chrome setup, explicit identity approval, reconnect guidance, and the unchanged
  Embedded/WinApp runtime requirements.
- Machine-readable package metadata declares Rust startup and endpoint markers,
  no Python requirement for Chrome, and no automatic fallback. The explicit
  Python migration selector remains documented until task 8.7.
- Package inspection rejects stale runtime metadata. A negative test changes the
  default back to Python and rebuilds the bundle hashes, proving that a valid
  checksum does not bypass the semantic compatibility check.
- Shared fixtures retain the legacy Python endpoint and add a credential-free
  Rust endpoint. The production endpoint reader is tested against both markers.
- CI now runs the existing isolated installed-package smoke. Release inventory
  validation and the nightly CI gate continue to use the same fail-closed
  compatibility contracts. Wire schema/protocol remain version 1; this stage
  introduces no runtime protocol change.

## Validation

All commands below passed:

```text
cargo build -p teshi-cli --locked
cargo test -p teshi-tui --locked cli::browser_endpoint::tests
cargo test -p teshi-browser-broker --locked protocol::tests
cargo clippy -p teshi-tui --locked --all-targets --all-features -- -D warnings
cargo fmt --all --check
node --test extension/teshi-bridge/tests/protocol.test.mjs extension/teshi-bridge/tests/network-capture.test.mjs
python -B scripts/test-browser-agent-package.py
python -B scripts/test_release_package.py
python -B scripts/test-release-workflow-contract.py
python -B scripts/test-nightly-workflow-contract.py
openspec validate migrate-chrome-browser-broker-to-rust --strict
git diff --check
```

Endpoint tests: 8 passed. Broker protocol tests: 8 passed. Extension tests:
40 passed. Package inventory tests: 8 passed. The final package smoke explicitly
set `TESHI_CLI` to the freshly built `target/debug/teshi.exe`; it copied that
binary to a temporary installed layout, started its internal Rust broker on an
ephemeral port with a bin-only PATH and private state, and checked Embedded
entry-point imports without checkout PYTHONPATH. Cleanup owns only its child
process. It does not alter real pairing, the shared broker, or project endpoints.

## Remaining acceptance boundaries

Task 8.4 still lacks an actual macOS build/archive/native inspection in the
retained packaging evidence. Structural Mach-O fixtures do not satisfy it.
This Windows session does not supply a macOS execution environment, and no
remote workflow or release was triggered. Windows/Linux release artifacts were
not rebuilt in this documentation/contract stage.

Task 8.7 explicitly requires every acceptance gate to pass before production
Python selection and dead Chrome-only code are removed. It remains unchecked;
`browser_service.py` still imports `browser_agent_broker.py`, so the packaged
import closure remains intact. Task 8.8 remains unchecked pending that removal,
fresh release-equivalent validation, and an updated final stage report.

No real Chrome, UI/UAC, WASM, Linux/macOS execution, full-workspace quality run,
installed-machine upgrade, commit, push, or publication is claimed by this stage.
