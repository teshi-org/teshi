## 1. Shared ACP configuration

- [x] 1.1 Move the reusable ACP definition, path, load, validation, atomic upsert, and removal operations into `teshi-acp` while preserving existing JSON compatibility.
- [x] 1.2 Add runtime `env_from` resolution with missing-variable errors that do not expose values.
- [x] 1.3 Adapt the TUI custom and Registry flows to the shared store without changing session-only selection behavior.

## 2. CLI management surface

- [x] 2.1 Add the `teshi acp list`, `show`, `add`, `remove`, and `config-path` command definitions and dispatch.
- [x] 2.2 Implement stable human output, JSON output, structured arguments, environment-reference parsing, executable resolution, and upsert behavior.

## 3. Documentation and verification

- [x] 3.1 Document the configuration path, JSON schema, environment-reference behavior, and CLI examples in the ACP architecture guide.
- [x] 3.2 Add focused tests for backward-compatible loading, validation and atomic mutation, environment resolution, CLI parsing/output, TUI shared-store behavior, and config preservation on failure.
- [x] 3.3 Run formatting, focused ACP/TUI/CLI tests, workspace check, and diff validation; record any environment-limited gates.
