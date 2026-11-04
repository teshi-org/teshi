# Test Strategy and Behavior Ownership

Teshi uses executable tests as the source of truth for behavior. There is no
parallel requirements/specification tree to keep synchronized with the code.

## Test layers

| Layer | Owns | Typical location |
| --- | --- | --- |
| Feature E2E | User-visible workflows and diagnostics across a real CLI, TUI, browser, or WinApp boundary | `features/`, `tests/steps/`, `crates/teshi-tui-runner/` |
| Rust unit tests | Pure parsing, validation, domain rules, rendering projections, and state transitions | `#[cfg(test)]` modules under `crates/` and `apps/` |
| Rust integration tests | Public CLI, process, daemon, protocol, persistence, and crate-boundary contracts | `*/tests/` and `apps/*/tests/` |
| Python and Node contract tests | Browser broker, extension, packaging, and other external-runtime boundaries | `resources/tests/`, `extension/teshi-bridge/tests/`, `scripts/` |

## Feature ownership

Every product Feature has a runner tag checked by
`scripts/check-feature-e2e-catalog.py`:

- `@cli` covers requirement-library and command-line workflows.
- `@validation-e2e` covers validation self-bootstrap through the built CLI.
- `@tui` covers real terminal behavior through the TUI runner.
- `@web-ui` covers browser UI behavior and requires a binding sidecar once bindings are confirmed.
- `@winapp` covers native Windows UIA behavior and requires a binding sidecar once bindings are confirmed.
- A header that says the work is not bound, or that bindings remain pending, keeps the runner tag without a fabricated sidecar.
- `@api` covers HTTP/API workflows.

The English and Chinese product Feature trees are paired where the workflow is
localized. Step text describes user intent; selectors, protocol envelopes, and
URLs belong in bindings or test fixtures rather than in the scenario prose.

## Adding or changing behavior

1. If a user can observe the behavior, add or update a Feature and execute it
   through its declared runner.
2. Cover the smallest deterministic rule with a Rust unit test.
3. Add an integration/contract test when the behavior crosses a process, crate,
   persistence, protocol, browser, extension, or packaging boundary.
4. Keep implementation rationale in the nearest architecture/development doc;
   do not create a second requirements document that can drift from the tests.

Feature coverage does not replace unit or integration coverage. Conversely,
unit and contract tests do not prove that a complete user workflow is usable;
the relevant layers should be kept together for a change.

## Local verification

```text
python scripts/check-feature-e2e-catalog.py
cargo run --locked -p teshi-cli -- check --all
cargo test --workspace --exclude teshi-web --locked
cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked
```

The CI workflow additionally runs the browser/extension Python and Node
contracts, the hosted UI transport check, validation CLI E2E, and platform
specific TUI/WinApp gates. See `features/README.md` and
`doc/development.md` for runner-specific commands.
