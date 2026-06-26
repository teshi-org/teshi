## ADDED Requirements

### Requirement: Desktop logging fix

The desktop app SHALL ensure `tracing::debug!` logs are written to disk.

- The logging initialization SHALL attempt `try_init()` first
- If `try_init()` returns Err (global subscriber already set), the system SHALL use `reinit()` to force-replace the existing subscriber
- The log level filter SHALL be "debug" to capture terminal diagnostics
- Log output SHALL be written to `{app_data_dir}/logs/teshi-desktop.log.YYYY-MM-DD`

#### Scenario: Tauri plugin already set a global subscriber

- **WHEN** Tauri plugins have already initialized a global tracing subscriber
- **THEN** `try_init()` returns Err
- **AND** the system calls `reinit()` to replace the subscriber
- **AND** `tracing::debug!` messages are written to the log file

#### Scenario: No prior global subscriber

- **WHEN** no prior global subscriber exists
- **THEN** `try_init()` returns Ok
- **AND** logging proceeds normally

### Requirement: Frontend terminal initialization diagnostics

The frontend SHALL log the full terminal initialization sequence to the browser console (desktop WebView).

- When the xterm instance is created, a `console.debug` log SHALL be emitted with instance ID
- When the `onData` handler is registered, a log SHALL be emitted
- When the `terminal-output` event listener is bound, a log SHALL be emitted
- When `spawnTerminal` is called and completes, logs SHALL be emitted with the terminal dimensions
- When a `terminal-exit` event is received, a log SHALL be emitted with the exit reason
- When a respawn is triggered (auto or manual), a log SHALL be emitted with the debounce state

#### Scenario: Full terminal initialization sequence is logged

- **WHEN** the Terminal tab is first opened
- **THEN** the following logs appear in sequence: xterm created → onData registered → event listener bound → spawn_terminal called → spawn_terminal completed

#### Scenario: Shell exit and respawn are logged

- **WHEN** the shell exits
- **THEN** a log is emitted with the exit event
- **AND** if auto-respawn is triggered, a log is emitted with the debounce state and backoff delay
