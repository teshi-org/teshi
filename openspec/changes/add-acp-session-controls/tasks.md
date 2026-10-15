## 1. Protocol configuration

- [x] 1.1 Add normalized session options with grouped select, boolean, unknown-type handling, and per-category legacy fallback tests.
- [x] 1.2 Preserve new/load metadata and implement validated config, mode, and model mutations with wire-shape tests.
- [x] 1.3 Normalize configuration notifications and verify session identity and authoritative replacement semantics.

## 2. Runtime state and lifecycle

- [x] 2.1 Separate connection/discovery from prompt startup and retain session configuration per conversation.
- [x] 2.2 Add serialized configuration commands, pending/error state, idle updates, stale-selection rejection, and timeout reconciliation.
- [x] 2.3 Make effective client permission policy observable and mutable while retaining pending permission decisions and environment overrides.
- [x] 2.4 Test reconnect, conversation isolation, failure recovery, dependent choices, and cancellation.

## 3. TUI controls

- [x] 3.1 Render bottom controls for agent settings and effective client approvals using display labels and terminal-cell widths.
- [x] 3.2 Add mouse and keyboard pickers with grouped options, descriptions, boolean choices, and narrow-layout overflow.
- [x] 3.3 Integrate discovery before the first prompt and loading/pending/busy/error presentation.
- [x] 3.4 Test rendering, click hit regions, keyboard navigation, Unicode, narrow terminals, and Native backend compatibility.

## 4. Validation and documentation

- [x] 4.1 Update ACP architecture and interaction documentation and run affected automated tests, formatting, native checking, and lint gates.
- [x] 4.2 Exercise a real configured ACP agent: discover options, switch supported values, verify the same session and subsequent prompt behavior.
- [x] 4.3 Verify visible TUI mouse and keyboard interactions in a real terminal and record acceptance limits.
