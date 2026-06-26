## Why

The embedded terminal in the desktop (Tauri) exhibits continuous scrolling (infinite output) after opening. The daemon/web mode works correctly — the root cause lies in Tauri-specific event delivery and terminal initialization timing on the desktop side. Additionally, debug logs are silently lost because `tracing_subscriber::try_init()` is preempted by Tauri's internal subscriber setup, making troubleshooting impossible.

## What Changes

- Fix desktop logging initialization to ensure `tracing::debug!` logs are written to disk
- Add output rate detection in the PTY reader: auto-kill/respawn shell when excessive output is produced in a short period
- Add frontend terminal initialization diagnostic logs (full sequence: xterm creation, event binding, spawn calls)
- Add debouncing on terminal-exit loops: prevent infinite respawn when shell exits/restarts frequently

## Capabilities

### New Capabilities
- `terminal-loop-detection`: PTY output rate monitoring and loop detection, automatic reset on anomaly
- `terminal-diagnostics`: Full-chain desktop terminal initialization logging

### Modified Capabilities
- (None — terminal currently has no formal spec)

## Impact

- `crates/teshi-runtime/src/terminal.rs` — Add rate detection in PTY reader thread
- `desktop/src-tauri/src/lib.rs` — Fix logging initialization
- `desktop/src/panels/FileTreeTerminalPanel.tsx` — Add frontend diagnostic logs, improve respawn debouncing
- `desktop/src-tauri/src/commands.rs` — Possibly add debug command to expose terminal status
