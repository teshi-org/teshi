# Terminal Send

## Purpose

Write text or keystrokes to the PTY without waiting for command completion, enabling interactive workflows where caller writes input and then reads the response separately.

## ADDED Requirements

### Requirement: Write text to PTY

The system SHALL accept a text string and write it to the PTY stdin immediately.

- The endpoint SHALL accept a JSON request body with a `data` string
- The system SHALL write the exact bytes of `data` to the PTY's writer handle
- The system SHALL flush the PTY writer after writing
- The response SHALL return immediately after the write/flush completes, without waiting for any shell output
- The `data` string SHALL be written as-is, without adding any newlines or other characters

#### Scenario: Send text returns immediately

- **WHEN** a request is sent with `{"data": "echo hello"}`
- **THEN** the system writes "echo hello" to the PTY
- **AND** returns `{"ok": true}` immediately
- **AND** does not wait for any output

#### Scenario: Send with newline flag appends \\n

- **WHEN** a request is sent with `{"data": "echo hello", "newline": true}`
- **THEN** the system writes "echo hello\\n" to the PTY
- **AND** returns immediately

### Requirement: Send on non-existent terminal

The system SHALL return an error if a send request arrives but no terminal session is active.

- Unlike exec, send SHALL NOT auto-spawn a terminal
- The response SHALL be `{"ok": false, "error": "terminal not spawned"}` with HTTP 400

#### Scenario: Send without active terminal returns error

- **WHEN** a send request arrives
- **AND** no terminal session exists
- **THEN** the response is an error
- **AND** the caller must spawn a terminal first (via exec, snapshot, or separate spawn call)

### Requirement: Send control characters

The system SHALL accept control character sequences in the data string for sending special keystrokes.

- The `data` field SHALL pass through raw bytes including control characters like `\\x03` (Ctrl-C), `\\x1b` (Escape), etc.
- A convenience `key` field MAY be used for named keys: `"ctrl_c"`, `"escape"`, `"enter"`, `"tab"`, `"up"`, `"down"`
  - When `key` is provided, the system SHALL convert the named key to the corresponding byte sequence
  - `"ctrl_c"` → `\\x03`, `"escape"` → `\\x1b`, `"enter"` → `\\r`, `"tab"` → `\\t`
  - Arrow keys (`up`, `down`, `left`, `right`) → corresponding CSI sequences

#### Scenario: Ctrl-C via named key

- **WHEN** a request is sent with `{"key": "ctrl_c"}`
- **THEN** the system writes `\\x03` to the PTY
- **AND** returns `{"ok": true}`

#### Scenario: Arrow key via named key

- **WHEN** a request is sent with `{"key": "up"}`
- **THEN** the system writes `\\x1b[A` to the PTY
- **AND** returns `{"ok": true}`
