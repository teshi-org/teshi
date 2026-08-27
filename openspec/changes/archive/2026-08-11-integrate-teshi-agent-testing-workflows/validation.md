# Validation record — 2026-08-10

## Passing automated gates

- `cargo fmt --all --check`
- `cargo check -p teshi-ui -p teshi-daemon -p teshi-desktop -p teshi-tui --locked`
- `cargo test --workspace --exclude teshi-web --locked`
- `cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings`
- `cargo test -p teshi-engine --locked embedded_cli` (3 tests)
- Targeted CLI parsing and operation-error tests for hyphen-leading lease tokens and non-zero JSON failures
- `python3 -m unittest discover -s resources/tests -p 'test_*.py' -v` (24 tests)
- `node --test extension/teshi-bridge/tests/*.test.mjs` (3 tests)
- `python3 scripts/test-browser-agent-package.py`
- `PATH=/home/love/.cargo/bin:$PATH bash scripts/run-web-ui-smoke.sh`
- `test ! -e apps/teshi-web-ui` plus a repository reference audit covering runtime,
  release, documentation, hook, and ignore paths
- `npx --yes @fission-ai/openspec@1.4.1 validate integrate-teshi-agent-testing-workflows --strict`
- Windows `cargo build -p teshi-cli --locked`
- PowerShell parser checks for `scripts/build-msi.ps1` and `scripts/build-teshi-web.ps1`

The GPUI WASM smoke gate built the nightly `wasm32-unknown-unknown` target, ran
`wasm-bindgen`, verified the `teshi-ui-runtime=gpui-wasm` marker and non-empty wasm,
and rejected React/Vite runtime markers. The React application and its npm tests
have been removed from the repository.

## Real Chrome and GPUI WASM validation

- Three extension instances (`manual-a`, `manual-b`, and `manual-c`) registered as
  independent ready sessions with distinct instance/window/tab identifiers.
- Discovery reported `ambiguous_browser_target: true` and
  `selected_session_id: null`; an untargeted `browser snapshot` returned
  `ambiguous_browser_target` and process exit code 1 without navigation.
- An explicit lease on `manual-c` navigated only target
  `fba3b8f8-c9c5-41e2-9641-c97c0628c07f/912202129/912202131` to the built GPUI
  shell. Snapshot title was `teshi — GPUI Web` and screenshot evidence showed all
  three profile cards plus the message that no profile is automatically chosen.
- The daemon same-origin inventory adapter returned the broker sessions, and the
  explicit tab-activation adapter returned `ok: true`, `queued: true`, and the same
  composite target.
- Broker restart/disconnect behavior preserved target identity and did not reuse a
  different profile. Unit/E2E tests additionally cover reverse-order replies,
  colliding tab IDs, lease recovery, stale pages, and screenshot isolation.
- Windows canonical project roots using the `\\?\` prefix are normalized before
  broker comparison; a regression test covers equivalence with ordinary drive paths.
- The validation lease was released and `manual-c` was restored to
  `https://example.com/` (`Example Domain`).

Evidence: `.teshi/evidence/browser-agent-46092-1786372938191-1.jpg`.

## Platform note

`teshi-web` remains intentionally excluded from native workspace commands because it
is wasm32-only. Its required target build is covered by `run-web-ui-smoke.sh` instead.
The only remaining compiler notice is the upstream future-incompatibility notice for
`proc-macro-error2 v2.0.1`; it is not a warning in Teshi code and does not fail gates.
