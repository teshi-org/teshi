## Context

Teshi's TUI consumes NDJSON runner events and `teshi run` supports explicit external runner configuration. The excluded legacy `runner/` references a deleted `tests/steps/tui` crate and is not usable. The engine's embedded terminal already uses portable-pty, but is a UI terminal panel rather than a test assertion backend. Existing dirty ACP/TUI changes must be preserved.

## Goals / Non-Goals

**Goals:** Drive arbitrary TUI executables using Rust; support typed launch/input/resize/screen/exit assertions; integrate scenario selection and reporting; test the actual Teshi executable with isolated projects; produce useful failure evidence.

**Non-Goals:** Browser or UIA emulation, shell command inference from prose, accepting unknown steps as passes, and replacing unrelated ACP changes.

## Decisions

1. Use `tui-test-rs` 0.1.0 in an isolated Cargo workspace at `crates/teshi-tui-runner`. Its published Rust API supplies in-process PTY sessions, terminal emulation, locators, keyboard input and lifecycle operations on Windows/Linux/macOS. `testty` is another Rust-native candidate; direct portable-pty plus vt100 would duplicate existing framework facilities. The selected framework passed a real Windows PTY fixture. Isolation is necessary because tui-test-rs requires unicode-width ^0.2.2 while Teshi's Ratatui 0.29 pins =0.2.0; the NDJSON process boundary avoids upgrading the application UI library for its testing backend. Build and test this crate explicitly with `--manifest-path crates/teshi-tui-runner/Cargo.toml`.
2. Keep automation separate from Gherkin intent. Store typed terminal actions in a feature sidecar, indexed by step source line, consistent with browser/WinApp binding concepts. Validate bindings before launching; unsupported or missing actions fail. Launch uses an executable and argument vector, never implicit shell interpolation.
3. Provide `teshi tui` execution and a native NDJSON runner adapter. Reuse existing run events and attachments so results are visible through existing reporting. Honor scenario identity, Background, Rule and partial execution boundaries; unsupported outline behavior must fail explicitly until implemented.
4. Scope a session to a scenario and close it on success and error. Assertions observe the rendered current screen, use bounded waits, and retain screen text when failing. Never search historical raw output to prove current visibility.
5. Self-tests launch the built Teshi CLI in isolated temporary project/data directories. Exercise navigation, editor/save, and exit through real input and assert visible state plus saved file content. Add CI commands and a reproducible developer invocation.

## Risks / Trade-offs

- Framework API/platform behavior may differ from documentation -> pin the dependency and run real PTY tests before adoption.
- Terminal rendering and asynchronous startup can be flaky -> fixed dimensions, visible-condition waits, bounded timeouts, and failure screen capture.
- Existing external runner overrides must retain precedence -> test explicit overrides and native TUI routing independently.
- Background and scenario line identities can be ambiguous -> use parsed AST identity and reject ambiguous bindings.

## Migration Plan

Add the capability without changing browser/WinApp configuration. Introduce and verify the backend, then the sidecar and runner adapter, then CLI/UI routing and self-tests. Remove only newly introduced routing if rollback is necessary.

## Verification decisions

- Windows and Linux real PTY fixtures pass input, key, repaint/current-screen, resize, timeout diagnostics, invalid launch, close/relaunch and actual process exit assertions. Cleanup tests retain a Windows process handle or observe the owned Unix PID, proving termination after timeout/drop and explicit close.
- Direct child exit uses `WaitExit` followed by `State.exited`; framework `ExpectExitCode` instead observes shell command markers. The key API rejects multi-character literal fallback so misspelled key names cannot silently type text.
- Windows canonical working directories are simplified before ConPTY launch; otherwise repeated runs after creating `.teshi` can produce an empty terminal. A regression covers canonical cwd startup and CLI failure/replay cycles.
- Feature Background, Rule-nested scenarios and partial execution are supported. Scenario Outlines, Rule Background and multiple Background blocks fail explicitly. Rule Background cannot be represented by the existing shared AST; no step is silently skipped to claim success.
- Both locale self-tests exercise real CLI routing and Explore's r action, verify case/step results in the rendered UI, edit/save through real input, assert process exit and compare the saved file contents. Linux navigation uses the supported direct tab shortcut rather than adjacent Escape/character bytes, which terminal input can interpret as Alt.
