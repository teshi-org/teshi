## Context

`teshi run` already supports an NDJSON runner contract and the repository has a requirement-library runner, but the validation acceptance Features are not executable. The target binary must be tested as a black box while the runner remains independent enough that a defect in `teshi check`, parser gating, or runner startup cannot make its own assertion pass.

The repository supports English and `zh-CN` self-test Features. The runner must work on the native CI platforms, use isolated temporary projects, and invoke the exact binary selected by the integration test rather than a PATH-installed Teshi.

## Goals / Non-Goals

**Goals:**

- Execute bilingual validation acceptance scenarios through the existing `teshi run` NDJSON protocol.
- Exercise the built Teshi CLI in child processes for source diagnostics, scope selection, warnings, binding gates, run gates, and replay preflight.
- Keep malformed source in generated fixture files, not in the executable test Feature itself.
- Verify runner non-start and state-preservation behavior with filesystem markers and `.teshi/active-step.json` snapshots.
- Provide a repeatable local/CI command and report actual scenario counts.

**Non-Goals:**

- Replacing core Rust tests for parser partial-AST behavior or editor rendering.
- Treating static architecture ownership checks as black-box CLI scenarios.
- Adding browser or WinApp bindings that require a live external target for this CLI-focused change.
- Making the validation runner depend on Teshi's internal validator API for assertions.

## Decisions

### 1. Use a dedicated validation runner

Add `teshi-validation-cli-runner` under `tests/steps`. It implements only the existing one-line request/NDJSON event protocol and validation-specific steps. The existing requirement runner remains focused on requirement-store behavior and its expected scenario count does not change.

### 2. Launch the target binary as a child process

The CLI integration test resolves `CARGO_BIN_EXE_teshi` (with the existing debug fallback), builds the validation runner, and passes `TESHI_BIN` to the outer run. The validation runner uses that path for every command. Each scenario owns a temporary project and app-data directory so project discovery, `.teshi` state, and environment variables cannot leak between cases.

### 3. Test invalid source as data

The executable Feature files remain syntactically valid. Given steps write explicit valid, warning-only, or malformed fixture Features into the scenario project. When steps invoke `check`, `steps`, `run`, or replay commands, Then steps inspect process status, human output, or JSON returned by the target binary.

### 4. Prove negative gates independently

The run-gate scenario passes the validation runner executable itself as the child runner and sets a marker path. A correct validation failure leaves the marker absent. The next-unbound scenario selects a valid step first, snapshots the active-step JSON, then attempts the invalid Feature and verifies the snapshot is unchanged.

### 5. Keep the Feature contract paired and focused

Add `@cli` plus a validation-specific tag to paired English/Chinese files. The integration test copies only those files to a temporary project and runs each locale directory with the dedicated runner. Existing abstract ownership/editor Features remain documentation/specification coverage and continue to be covered by Rust or source-contract tests.

## Risks / Trade-offs

- [The runner could accidentally use an installed Teshi] → Require and validate `TESHI_BIN`, pass the integration binary explicitly, and fail if it does not exist.
- [The target run gate could launch the runner before reporting validation errors] → Use a marker file created only by the nested runner process and assert it remains absent.
- [Fixtures or `.teshi` state could leak across scenarios] → Create one `World` with fresh `tempdir` values per case and never write repository-local state.
- [Bilingual step text can drift] → Keep one implementation branch per English/Chinese pair and execute both locales in CI.
- [The same target binary still relies on its own parser to discover the test Feature] → Keep the test Feature valid and use external fixture source to exercise malformed-input validation; lower-level parser behavior remains separately tested.

## Migration Plan

1. Add the runner crate and workspace member.
2. Add paired executable validation Features and the CLI integration harness.
3. Run focused validation E2E, then the native Rust quality gates and `teshi check --all`.
4. Add the focused command to CI without removing existing unit/integration gates.

Rollback is limited to removing the new runner/integration command and tags; production validation behavior is unchanged.

## Open Questions

- Browser/Web and native WinApp acceptance should get separate live-target runners when their environments are available; this change intentionally covers the deterministic CLI/preflight surface.
