# Verification

## Requirements and authoritative evidence

| Requirement | Evidence |
| --- | --- |
| Rust terminal runner using an existing framework | Isolated `crates/teshi-tui-runner/Cargo.toml` and lockfile pin `tui-test-rs` 0.1.0; `src/lib.rs` invokes its real in-process PTY API. |
| Launch, text, keys, resize and current-screen assertions | `tests/terminal.rs` drives an executable fixture, replaces its screen and verifies the old text is absent. The backend and CLI tests pass on Windows and Linux. |
| Invalid bindings fail before launch | `scenario::tests` cover missing/stale/unconfirmed/duplicate bindings, unknown actions, invalid keys, resize dimensions, bounded timeouts, repeated launch, identity and partial boundaries. |
| Scenario selection and standard reporting | `tests/cli.rs` invokes the actual `teshi tui` and `teshi run` binaries with relative paths, selects one scenario and verifies standard case/step events. |
| External runner precedence and failure exit | CLI tests exercise explicit command, environment and `teshi.toml` configuration with an intentionally unusable automatic TUI runner. Failed cases produce nonzero CLI exits in every route. Malformed configuration and unknown selections fail before launch. |
| Lifecycle and failure evidence | Windows tests retain a handle to the owned fixture process; Unix tests observe its PID. Both prove termination after assertion failure/drop and explicit close. CLI/protocol failures attach readable current-screen files. |
| Existing TUI result consumption | The actual Teshi self-test presses r in Explore and asserts `Status: Passed` and the passed step text in the rendered details panel. An App contract test parses failed runner events and verifies step failure, duration and screen attachment retention. |
| Teshi's own TUI in tests | Paired `features/en-US/tui_editor.feature` and `features/zh-CN/终端功能编辑.feature` cover navigation and run/edit/save/exit as separate scenarios. The harness launches actual Teshi executables in isolated project/data roots and compares the saved feature file exactly. Both locales pass on Windows and Linux. |
| CI and developer invocation | `.github/workflows/ci.yml` has Windows/Linux terminal jobs. Both run normal backend tests and explicitly run the ignored integration suite with the built CLI. `doc/development.md` and the runner README give the same reproducible commands. |

## Windows quality gates

Passed on the final implementation:

```text
cargo fmt --all --check
cargo check --workspace --exclude teshi-web --locked
cargo test --workspace --exclude teshi-web --locked
cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings
cargo doc --workspace --exclude teshi-web --locked --no-deps --document-private-items
cargo fmt --manifest-path crates/teshi-tui-runner/Cargo.toml --check
cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked
cargo clippy --manifest-path crates/teshi-tui-runner/Cargo.toml --locked --all-targets -- -D warnings
```

With `TESHI_SELF_TEST_BIN` pointing to the built Windows CLI, all four CLI
integration tests and both locale self-regressions pass when explicitly enabled
with `cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked -- --ignored`.
The locale tests execute two BDD scenarios each, including the actual Explore
runner path and saved-file assertions.

Both Feature files pass Teshi source validation with zero errors, warnings and
suggestions. `python scripts/check-feature-e2e-catalog.py` passes. OpenSpec strict
validation and `git diff --check` pass.
The final consolidated runner invocation used `-- --include-ignored` and passed
all 19 test functions with zero ignored on both Windows and Linux (three are
child-fixture entrypoints; the rest assert behavior).

Local gate logs are under `target/tui-validation-20261014-083406` (not committed).
The full Clippy gate initially found an unused `AcpEvent` import in an existing
untracked ACP test. Only that import was removed; the test content was preserved.
Documentation generation succeeds with four pre-existing rustdoc warnings in
agent registry, core mindmap, editor-buffer links and the browser CLI URL.

## Linux execution

Verified with a native Linux build in Ubuntu WSL using an isolated Rust toolchain
and Cargo targets under `/var/tmp/teshi-tui-verification-20261014`:

- Runner real PTY/protocol/binding tests pass, including Unix process cleanup.
- Runner strict Clippy passes with all targets.
- `cargo build -p teshi-cli --locked` produces a native Linux Teshi executable.
- All four public CLI integration tests and both locale TUI self-tests pass
  against that Linux executable, including nested Explore execution and saved
  file comparisons.

The locked Zed Git objects were copied read-only from the existing Windows Cargo
cache into the isolated Linux cache; the exact pinned commit was verified first.
The initial network fetch was deliberately stopped before switching caches.
No Windows build artifacts were reused as Linux executables.

## Scope and limitations

- The runner is an independently installed component because its Unicode
  dependency conflicts with the application's Ratatui version. Install it with
  the documented Cargo command or supply its path through `TESHI_TUI_RUNNER` or
  `--runner-cmd`; it is resolved beside Teshi or through PATH otherwise.
- Bindings are explicit JSON. Feature Background, ordinary Rule scenarios and
  partial step execution are covered. Outlines, Rule Background and multiple
  Background blocks fail explicitly rather than executing a partial scenario.
- CI configuration is verified locally; a remote workflow run and publication
  are not claimed. Native workspace gates exclude the unchanged wasm-only
  `teshi-web` crate. macOS execution is not claimed.
