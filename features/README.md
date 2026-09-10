# Product E2E Features

Every `.feature` file under this directory describes a user-observable
end-to-end behavior and has an explicit verification route. A Feature must
have an explicit runner owner:

- `@validation-e2e` for the validation CLI self-bootstrap runner
- `@cli` for requirement-library CLI E2E
- `@web-ui` for browser/UI E2E and verified replay
- `@api` for HTTP API E2E

`@web-ui` files also require a matching `*.bindings.json` artifact. The native
CI job checks the catalog and runs the CLI self-bootstrap; browser replay is
run through Teshi's browser workflow. A Feature without a bound browser
workflow is not added here just to document an internal protocol contract.

Do not put Core unit-test intent, architecture ownership, implementation
boundaries, CI packaging rules, or transport-internal invariants here. Keep
those in OpenSpec, Rust integration tests, workflow contract tests, or
documentation. If a user-visible behavior depends on one of those details,
describe the observable result instead.

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
