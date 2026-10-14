## 1. Terminal backend

- [x] 1.1 Evaluate existing Rust frameworks with a real Windows PTY fixture and record the backend decision.
- [x] 1.2 Add an isolated Rust crate with typed launch, input, key, resize, screen and exit APIs, bounded waits and session cleanup.
- [x] 1.3 Verify successful interaction, current-screen semantics, timeouts, invalid actions and cleanup with real PTY tests.

## 2. Scenario execution

- [x] 2.1 Define and validate TUI sidecar bindings, including missing steps, scenario identity, Background, Rule and partial execution behavior.
- [x] 2.2 Implement the Rust NDJSON runner adapter with standard case events and failure screen attachments.
- [x] 2.3 Integrate `teshi tui` and TUI scenario routing with existing CLI/UI execution while preserving explicit runner overrides.
- [x] 2.4 Test scenario selection, failure propagation, evidence and CLI/UI event compatibility.

## 3. Teshi self-tests and delivery

- [x] 3.1 Add BDD self-regressions for actual Teshi tab navigation, editor/save and exit using isolated fixtures.
- [x] 3.2 Run self-regressions through the supported Teshi runner path and verify saved file contents.
- [x] 3.3 Add CI coverage and development documentation with reproducible commands and platform limitations.
- [x] 3.4 Run formatting, relevant tests, native workspace checks and strict Clippy; audit every requirement against current evidence.
