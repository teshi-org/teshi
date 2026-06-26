## 1. Fix desktop logging initialization

- [x] 1.1 Change logging init in `lib.rs` from `try_init()` to attempt `try_init()` first, fallback to `reinit()` on failure
- [x] 1.2 Ensure `tracing::debug!` in the desktop Tauri process correctly writes to rolling log files under `{app_data_dir}/logs/`

## 2. PTY output rate detection

- [x] 2.1 Add rate counter (per 100ms sliding window, byte count) in `terminal.rs` PTY reader thread
- [x] 2.2 When rate exceeds 1MB/s for 200ms, mark loop state and notify forwarder thread
- [x] 2.3 When forwarder detects loop flag, skip `terminal-output` emit, emit `terminal-loop-detected` event instead
- [x] 2.4 Handle `terminal-loop-detected` event: kill shell, show prompt message in frontend, auto-respawn with exponential backoff
- [x] 2.5 Implement backend exponential backoff delay logic (1s, 2s, 4s, 8s, 16s, cap 30s), reset backoff after 30s+ of normal shell operation

## 3. Frontend respawn debouncing

- [x] 3.1 Add `lastSpawnTimeRef` and spawn frequency counter in `FileTreeTerminalPanel.tsx`
- [x] 3.2 Skip auto-spawn in `ensureShellSpawned` if less than 3s since last spawn
- [x] 3.3 Stop auto-respawn when shell exits ≥3 times within 10s, show prompt, keep manual "Restart shell" button functional

## 4. Frontend terminal initialization diagnostic logs

- [x] 4.1 Add `console.debug` logs at: xterm creation, onData registration, terminal-output event binding, spawnTerminal call/completion, terminal-exit event
- [x] 4.2 Log debounce state and backoff delay when respawn is triggered (auto or manual)
