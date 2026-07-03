## Why

teshi currently has an embedded terminal panel (Tauri/Web xterm.js), but lacks a programmatic interface for AI agents or CLI tools to control an interactive terminal. The existing `teshi browser` and `teshi winapp` provide automated CLI for browsers and Windows applications, but terminal automation is still a gap. The `teshi terminal` subcommand fills this gap, enabling agents (Chrys, Claude Code, etc.) to read/write terminals in real-time via CLI commands, obtain structured screen grids, and execute commands with result waiting.

## What Changes

- **New `teshi terminal` CLI subcommand** — terminal automation commands analogous to `teshi browser`/`teshi winapp`, outputting JSON for agent consumption
- **New VTE screen grid parser** — integrate `vte` crate into `teshi-runtime` to parse raw PTY output into structured row/col grids, with dirty-row tracking and incremental reads
- **New sidecar process** — standalone Rust binary with WebSocket server, PTY management, and VTE grid parsing
- **New process state detection** — detect command states: Running / Idle / WaitingForInput / Exited
- **WebSocket Debug Viewer (optional)** — live terminal view in browser

## Capabilities

### New Capabilities
- `terminal-snapshot`: Read the current terminal screen as a structured grid (rows/cols/chars/attributes/cursor/state), supporting full and incremental reads
- `terminal-status`: Query terminal session state (process state, has_new_content, dimensions), low-cost polling
- `terminal-exec`: Write a command to PTY, wait for completion, return final screen grid and exit code
- `terminal-send`: Write text to PTY (one-way, no output waiting)
- `terminal-command-cli`: Register `teshi terminal` subcommand, communicate with sidecar via WebSocket, output JSON to stdout

### Modified Capabilities
- (None — existing terminal-loop-detection and terminal-diagnostics are unaffected, still embedded panel behavior)

## Impact

- **New dependency**: `vte = "0.15"` in `crates/teshi-runtime/Cargo.toml`
- **Modified crate**: `crates/teshi-runtime` (new `screen.rs` module)
- **New crate**: `crates/teshi-terminal-sidecar` (standalone binary)
- **Modified crate**: teshi CLI binary (`src/cli/mod.rs`, new `src/cli/terminal.rs`, `src/main.rs`)
- **Not affected**: frontend code (xterm.js / Tauri panel unchanged), existing specs unchanged
