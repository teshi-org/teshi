# Terminal Status

## Purpose

Provide lightweight terminal session status queries for low-overhead polling by CLI tools and AI agents, enabling callers to determine if the shell is ready for the next command without parsing the full screen grid.

## ADDED Requirements

### Requirement: Process state detection

The system SHALL detect and report the current process state of the shell running inside the PTY.

- The system SHALL report one of the following states:
  - `running`: PTY output has been received within the last 500ms
  - `idle`: No PTY output for 500ms+, the shell prompt is not currently visible on the last line
  - `waiting_input`: No PTY output for 500ms+, and the last line of the screen matches a shell prompt pattern
  - `exited`: The PTY child process has terminated
- The status response SHALL include the process state in a `state` field
- When state is `exited`, the response SHALL include the exit code when available, or `null` if unknown
- Prompt detection SHALL match common shell prompt patterns: `$ `, `# `, `> `, `% `, `❯ `, `PS C:\\` and variations
- Prompt detection SHALL consider both the raw cell content and the cursor position (cursor at or near end of prompt line)

#### Scenario: Running state after command execution starts

- **WHEN** text is written to the PTY (e.g., "npm install\\n")
- **AND** the shell begins producing output
- **THEN** a status query within 500ms returns `state: "running"`

#### Scenario: Waiting for input state when prompt is detected

- **WHEN** a command completes and the shell displays a new prompt
- **AND** at least 500ms have passed since the last PTY output
- **THEN** a status query returns `state: "waiting_input"`

#### Scenario: Exited state when shell terminates

- **WHEN** the shell process exits (e.g., via `exit` command)
- **AND** the PTY reader detects EOF
- **THEN** a status query returns `state: "exited"`
- **AND** the exit code field is populated

### Requirement: Has-new-content flag

The status response SHALL include a boolean `has_new_content` flag indicating whether the screen content has changed since the last snapshot or status query.

- The flag SHALL be reset to `false` after each snapshot request
- The flag SHALL be reset to `false` after each status request
- Any VTE-parsed character or attribute change SHALL set the flag to `true`

#### Scenario: Status shows new content flag

- **WHEN** a status query returns `has_new_content: true`
- **THEN** the caller can optionally issue a more expensive snapshot request
- **AND** a subsequent status query returns `has_new_content: false` (reset by the first status)

### Requirement: Lightweight status response

The status response SHALL be a lightweight JSON object suitable for high-frequency polling (e.g., every 200-500ms).

- The status response SHALL contain: `state`, `has_new_content`, `rows`, `cols`, `exit_code` (when applicable)
- The status response SHALL NOT include the full screen grid or cell data
- The status response SHALL be processable in under 1ms (no blocking I/O, no full grid copy)

#### Scenario: Status response structure

- **WHEN** a status request is made while the shell is idle
- **THEN** the response format is:
  ```json
  { "ok": true, "state": "waiting_input", "has_new_content": true, "rows": 24, "cols": 80, "exit_code": null }
  ```
