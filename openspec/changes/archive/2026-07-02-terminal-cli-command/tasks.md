## 1. Dependency & Module Setup

- [x] 1.1 Add `vte = "0.15"` dependency to `crates/teshi-runtime/Cargo.toml` (ScreenGrid module lives here)
- [x] 1.2 Create `crates/teshi-runtime/src/screen.rs` module with `ScreenGrid` struct, `GridPerformer` (impl `vte::Perform`), `ProcessState`, `Cell`
- [x] 1.3 Register `screen` module in `crates/teshi-runtime/src/lib.rs` and export `ScreenGrid`, `ProcessState`, `Cell`

## 2. VTE Screen Grid Implementation

- [x] 2.1 Implement `Cell` struct with `char`, `bold`, `dim`, `italic`, `underline`, `fg`, `bg` fields
- [x] 2.2 Implement `GridPerformer` with `vte::Perform` trait: `print()`, `execute()`, `csi_dispatch()`, `esc_dispatch()`, `osc_dispatch()`, `hook()`, `put()`, `unhook()`
- [x] 2.3 Implement cursor movement handlers: CUU, CUD, CUF, CUB, CUP, CU, CPR, SCP, RCP
- [x] 2.4 Implement erase handlers: EL (0/1/2), ED (0/1/2/3), DCH, ICH
- [x] 2.5 Implement SGR handler: bold, dim, italic, underline, colors (3-bit, 8-bit, 24-bit), reset
- [x] 2.6 Implement line feed, carriage return, tab, backspace, bell
- [x] 2.7 Implement scroll: new line at bottom pushes scrollback, scroll up/down sequences
- [x] 2.8 Implement dirty row tracking with `dirty: Vec<bool>` and reset mechanism
- [x] 2.9 Implement resize handler: reallocate grid, clamp cursor, mark all rows dirty
- [x] 2.10 Implement `ScreenGrid::snapshot(&self, full: bool) -> Value` returning JSON grid
- [x] 2.11 Implement `ScreenGrid::clear_dirty(&self)` to reset dirty flags

## 3. Process State Detection

- [x] 3.1 Implement `ProcessState` enum: `Spawned`, `Running`, `Idle`, `WaitingForInput`, `Exited(i32)`
- [x] 3.2 Implement state transition logic in `ScreenGrid` based on output recency (500ms threshold)
- [x] 3.3 Implement prompt pattern detection on last grid line for `WaitingForInput` state
- [x] 3.4 Implement `last_output_at: Instant` tracking for timing-based state transitions
- [x] 3.5 Implement `has_new_content: bool` flag and reset mechanism

## 4. Create Terminal Sidecar Crate

- [x] 4.1 Create `crates/teshi-terminal-sidecar/` with `Cargo.toml`: dependencies `portable-pty`, `vte`, `tokio`, `tokio-tungstenite`, `serde`, `serde_json`, `anyhow`, `teshi-runtime` (for ScreenGrid)
- [x] 4.2 Create `crates/teshi-terminal-sidecar/src/main.rs` entry point: parse args, bind TCP, write cdp-endpoint.json, start WS server
- [x] 4.3 Implement async WebSocket server using `tokio-tungstenite` on random port (`127.0.0.1:0`)
- [x] 4.4 Implement JSON command dispatcher: match `cmd` field and route to handler
- [x] 4.5 Implement `start_pty()`: spawn shell via `portable-pty::NativePtySystem`, set up reader thread → VTE parser → ScreenGrid
- [x] 4.6 Implement `handle_snapshot()`: read ScreenGrid, return JSON grid
- [x] 4.7 Implement `handle_status()`: return ProcessState + has_new_content + dimensions
- [x] 4.8 Implement `handle_send()`: write data to PTY writer, return ok
- [x] 4.9 Implement `handle_exec()`: write command + newline, poll ScreenGrid until state transitions to WaitingForInput/Idle, return snapshot
- [x] 4.10 Implement `handle_resize()`: resize PTY master and ScreenGrid
- [x] 4.11 Implement `handle_kill()`: stop PTY session, reset ScreenGrid
- [x] 4.12 Implement cdp-endpoint.json write on server start
- [x] 4.13 Handle cleanup on Ctrl+C / SIGINT: stop PTY, remove cdp-endpoint.json
- [x] 4.14 Register sidecar crate in workspace `Cargo.toml` members

## 5. Wire VTE into PTY Reader Thread

- [x] 5.1 In sidecar's `start_pty()`: create `Arc<Mutex<ScreenGrid>>`, spawn reader thread feeding `vte::Parser`
- [x] 5.2 Reader thread: `reader.read(buf)` → `parser.advance(performer, buf)` → ScreenGrid updated
- [x] 5.3 No base64/events emit needed (sidecar is not the GUI path)

## 6. CLI Subcommand Implementation

- [x] 6.1 Create `src/cli/terminal.rs` with `handle_terminal_command(action)` dispatch
- [x] 6.2 Implement `serve_embedded()`: spawn sidecar binary as child process, wait for Ctrl+C
- [x] 6.3 Implement `snapshot()`, `status()`, `exec()`, `send()`, `resize()`, `kill()` handler functions
- [x] 6.4 Each handler: read `.teshi/cdp-endpoint.json` → `send_sidecar_command_with_timeout` → `print_json_response`
- [x] 6.5 Reuse `send_sidecar_command_with_timeout` from `teshi_runtime::sidecar` (imported into CLI crate)
- [x] 6.6 Reuse `read_cdp_endpoint` from `src/cli/browser_endpoint.rs`
- [x] 6.7 Implement `ensure_ok()` / `print_json_response()` pattern (same as browser/winapp)

## 7. Register CLI in Teshi Binary

- [x] 7.1 Add `TerminalCommand` enum with subcommand variants (`ServeEmbedded`, `Snapshot`, `Status`, `Exec`, `Send`, `Resize`, `Kill`) in `src/cli/mod.rs`
- [x] 7.2 Add `Terminal { action: TerminalCommand }` variant to top-level `Command` enum
- [x] 7.3 Add dispatch arm `Some(cli::Command::Terminal { action }) => return cli::terminal::handle_terminal_command(&action);` in `src/main.rs`
- [x] 7.4 Add corresponding args structs with clap derive attributes

## 8. Build & Integration

- [x] 8.1 `cargo build` — verify workspace compiles (sidecar binary + teshi CLI)
- [x] 8.2 Verify `teshi terminal serve-embedded` starts and writes cdp-endpoint.json
- [x] 8.3 Verify `teshi terminal snapshot` returns JSON grid from running sidecar
- [x] 8.4 Verify `teshi terminal exec "echo hello"` returns snapshot with "hello" in output
- [x] 8.5 Verify `teshi terminal status` returns process state
- [x] 8.6 Verify `teshi terminal send "echo hello\n"` followed by snapshot returns correct output
- [x] 8.7 Verify `teshi terminal resize 120 40` followed by snapshot shows new dimensions
- [x] 8.8 Verify `teshi terminal kill` stops shell and resets session
- [x] 8.9 Verify error when no sidecar running (cdp-endpoint.json missing)
- [x] 8.10 Verify error when cdp-endpoint.json mode is not "terminal"

## 9. Backward Compatibility

- [x] 9.1 Verify existing embedded terminal panel (xterm.js / Tauri) still works: base64 event path unchanged
- [x] 9.2 Run existing tests (`cargo test`) to confirm no regressions
- [x] 9.3 Verify `teshi browser` and `teshi winapp` still work (no shared code changes affect them)
