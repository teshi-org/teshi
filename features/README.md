# Product E2E Features

Every `.feature` file under this directory describes a user-observable
end-to-end behavior and has an explicit verification route. A Feature must
have an explicit runner owner:

- `@validation-e2e` for the validation CLI self-bootstrap runner
- `@cli` for requirement-library CLI E2E
- `@tui` for real terminal/TUI E2E
- `@web-ui` for browser/UI E2E and verified replay
- `@api` for HTTP API E2E
- `@winapp` for native Windows UIA regressions replayed with `teshi winapp`

`@web-ui` and `@winapp` files require a matching `*.bindings.json` once
bindings are confirmed. A header that says the work is `not bound`, or that
live bindings `remain pending`, keeps the runner tag and does not fail this
catalog for a missing sidecar. Confirmed bindings are still not invented.
The native CI job checks the catalog and runs the CLI self-bootstrap; browser
replay is run through Teshi's browser workflow. A Feature without a bound
browser workflow is not added here just to document an internal protocol
contract.

`@winapp` replay still needs a live Windows target. Those files are cataloged
in CI but are not replayed on the Linux quality job; use `teshi winapp replay`
on Windows with the attached `teshi-desktop` target.

Do not put Core unit-test intent, architecture ownership, implementation
boundaries, CI packaging rules, or transport-internal invariants here. Keep
those in Rust unit/integration tests, workflow contract tests, or
documentation. If a user-visible behavior depends on one of those details,
describe the observable result instead. The complete ownership rule is in
[`doc/test-strategy.md`](../doc/test-strategy.md).

## Recorded execution results

Run the matrix against a source-built CLI and its sibling runner binaries:

```text
python scripts/run-feature-matrix.py --teshi path/to/teshi
python scripts/run-feature-matrix.py --teshi path/to/teshi --feature features/en-US/tui_conversation_history.feature
```

Each invocation creates `.teshi/feature-results/runs/<run-id>/matrix.json`,
per-Feature JSON/logs, and available case evidence. Previous runs remain intact.
An explicit `--output-dir` must be empty. Both failed and blocked Features make
the matrix exit unsuccessfully. Reports distinguish unavailable targets from
executed assertion failures.

Project-switch Features use checked-in `*.tui.template.json` bindings. The
matrix expands their binary and fixture-root tokens into a new isolated
directory per scenario. Run them through the matrix, or explicitly select
`python scripts/run-tui-sidecar-feature.py` as the external runner with
`TESHI_SELF_TEST_BIN` and `TESHI_GENERIC_TUI_RUNNER` set to source-built binaries.
Templates retain source step text and line numbers; stale bindings fail before
launch. Generated `*.tui.json` files stay in fixture directories.

Browser cases replay their background independently through `browser replay
--scenario <name>`, which rejects missing source-step bindings before dispatch.
Set `TESHI_BROWSER_PROJECT` to an isolated project containing an identical
`features/` tree and its own live browser endpoint; do not reuse a WinApp endpoint.
The local GPUI WASM test URL requires a functioning WebGPU adapter. The optional
`TESHI_EMBEDDED_SOFTWARE_WEBGPU=1` setting enables software-adapter launch flags
only for an explicitly configured embedded test browser.

Live CodeBuddy cases require the installed `codebuddy-code` profile and
`TESHI_CODEBUDDY_LIVE=1`. Credentials resolve in the proxy at process launch and
are excluded from generated sidecars and protocol evidence. The trace records
protocol metadata, and reply assertions use response-only markers in the real PTY.

The driving Feature must remain valid. Invalid or incomplete source used as a
test input belongs in an isolated fixture created by the runner; the scenario
still describes the user-visible command and diagnostic result.

The catalog check verifies ownership and the relevant runner/replay must pass
before treating a Feature as covered:

```text
python scripts/check-feature-e2e-catalog.py
cargo run --locked -p teshi-cli -- check --all
cargo test --locked -p teshi-cli --test validation_cli_bdd -- --nocapture
```

The repository-level test layers and their ownership are summarized in
[`doc/test-strategy.md`](../doc/test-strategy.md).
