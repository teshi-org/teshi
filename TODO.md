# TODO

Items recorded on 2026-10-13. These requirements are pending planning and implementation.

- [ ] Centralize runtime-generated files in a dedicated storage directory. Do not place runtime JSON files directly in the current working directory or in hidden directories beneath it. Plan project-level and global configuration, including storage locations and scope.
- [ ] Add drag-and-drop support to Teshi WinApp automation.
- [ ] Migrate the Teshi WinApp implementation to Rust.
- [ ] Expose Teshi's capabilities through MCP for use by ACP agents.
- [ ] Unify WinApp and Browser capabilities and interfaces as much as possible to simplify MCP calls.
- [ ] Add automated regression tests in `crates/teshi-agent-runtime/tests/acp_backend.rs` for ACP history restore fixes:
  - `session/load` with large history replay (>256 `session/update` frames) completes without channel backpressure deadlock and mutes replay frames from UI events.
  - Submitting the first turn before `controls.ready` becomes `true` does not prepend `[Restored conversation history]` when `session/resume` or `session/load` succeeds, and still prepends it when restore falls back to `session/new`.
  - In-place `AcpAgentBackend::connect()` reconnect after worker exit carries forward the latest confirmed remote `session_id` into `restore_session_id`.

