## Context

The desktop Tauri embedded terminal (`crates/teshi-runtime/src/terminal.rs`) uses `portable-pty` to create a ConPTY and spawn pwsh.exe. The PTY reader thread reads output, which is dispatched by the forwarder thread via `RuntimeEvents::emit` as `terminal-output` events. The daemon mode (WebSocket dispatch) works correctly, while the desktop mode (Tauri `handle.emit` dispatch) exhibits continuous scrolling.

The current logging uses `tracing_subscriber::fmt().try_init()`, but Tauri 2.x may set a global tracing subscriber at an earlier stage, causing `try_init()` to fail silently — all `tracing::debug!` logs are lost.

## Goals / Non-Goals

**Goals:**
- Fix desktop logging initialization so debug logs are written to disk
- Add PTY output rate detection: auto-kill/respawn when loop output is detected
- Add frontend diagnostic logs for terminal event binding and spawn timing
- Add respawn debouncing when shell exits frequently

**Non-Goals:**
- Do not change the basic architecture of PTY reader and forwarder
- Do not modify daemon mode terminal behavior
- Do not introduce new external dependencies

## Decisions

### 1. Logging initialization: `try_init` → fallback to `reinit`

`tracing_subscriber::fmt().try_init()` returns Err if a global subscriber already exists. Tauri 2.x's `tauri-plugin-shell` or other plugins may have already set one. Approach: try `try_init` first, and on failure call `reinit()` (force replace).

**Alternative considered**: Use `init()` directly (panics) — rejected, because the Tauri plugin's subscriber may carry necessary configuration.

### 2. Output rate detection: counting in the reader thread

Add to the PTY reader thread (`terminal.rs` reader loop):
- Count bytes read in each 100ms window
- If the threshold (e.g., 1MB/s) is exceeded, mark loop state
- The forwarder thread checks the loop flag, skips emit, and notifies `terminal-exit`
- After shell is killed, add exponential backoff delay before frontend respawn (1s, 2s, 4s... max 30s)

**Alternative considered**: Detect in the forwarder thread — rejected, because data has already left the reader and the shell cannot be killed in time.

### 3. Frontend respawn debouncing

In `FileTreeTerminalPanel.tsx`:
- Add `lastSpawnTimeRef` (millisecond timestamp) alongside `shellSpawnedRef`
- In `ensureShellSpawned`, skip if less than 3 seconds since last spawn
- In shell-exit handler, record exit time; if ≥3 exits within 10 seconds, stop auto-respawn

### 4. Frontend diagnostic logs

Add `console.debug` logs in the terminal initialization useEffect:
- xterm instance created
- terminal-output event bound
- spawn_terminal call started/completed
- terminal-exit event received

## Risks / Trade-offs

- [Rate threshold false positive] → Threshold set to 1MB/s; normal shell prompts and command output are far below this, only loop output triggers it
- [reinit may cause log duplication] → `reinit` replaces the existing subscriber, functionality is unaffected, only log destination changes
- [Frontend debouncing skips legitimate operations] → 3-second window is short, manual restart button is unaffected
