# Terminal Snapshot

## Purpose

Provide a structured snapshot of the current terminal screen content for programmatic consumption by AI agents and CLI tools, by parsing raw PTY output into a row/col character grid with cursor position and attributes.

## ADDED Requirements

### Requirement: Screen grid snapshot

The system SHALL provide a snapshot of the current terminal screen as a structured grid when requested via the daemon HTTP API.

- The snapshot SHALL include the full visible grid: rows × cols of cells
- Each cell SHALL include the character (`char`) and formatting attributes (`bold`, `dim`, `italic`, `underline`, `fg` color, `bg` color)
- The snapshot SHALL include cursor position (`row`, `col`)
- The snapshot SHALL include the terminal dimensions (`rows`, `cols`)
- The snapshot SHALL include the current process state (`running`, `idle`, `waiting_input`, `exited`)
- The response SHALL be a JSON object

#### Scenario: Full snapshot returns complete grid

- **WHEN** a request is made to the snapshot endpoint
- **THEN** the response returns a JSON object with `rows`, `cols`, `cursor`, `state`, and `cells` fields
- **AND** the `cells` array has exactly `rows` elements, each with `cols` elements

#### Scenario: Snapshot when no terminal is spawned

- **WHEN** a request is made to the snapshot endpoint
- **AND** no PTY session has been started yet
- **THEN** the response SHALL return `{"ok": false, "error": "terminal not spawned"}` with HTTP 400

### Requirement: Screen grid update via VTE parser

The system SHALL parse raw PTY output into a structured screen grid using the `vte` crate's ANSI parser.

- All printable characters SHALL be placed at the current cursor position in the grid
- Cursor movement CSI sequences (`CUU`, `CUD`, `CUF`, `CUB`, `CUP`, etc.) SHALL update the tracked cursor position
- Line feeds and carriage returns SHALL advance the cursor row and reset column appropriately
- Erase sequences (`EL`, `ED`) SHALL clear the corresponding cells in the grid
- Scroll sequences SHALL shift rows in the scrollback buffer
- SGR (Select Graphic Rendition) sequences SHALL update cell attributes (bold, dim, italic, underline, foreground/background color)
- OSC sequences for window title SHALL be captured but not rendered in the grid
- Unsupported or unknown sequences SHALL be safely ignored without crashing the parser

#### Scenario: Basic text output populates the grid

- **WHEN** the shell outputs "hello\\nworld"
- **THEN** row 0 cells contain "hello" starting at column 0
- **AND** row 1 cells contain "world" starting at column 0

#### Scenario: Cursor movement sequences move the tracked cursor

- **WHEN** the shell outputs "abc\\x1b[2DXY"
- **THEN** the grid contains "aXY" at the cursor row, columns 0-2

#### Scenario: SGR sequences set cell attributes

- **WHEN** the shell outputs "\\x1b[1mbold\\x1b[0mnormal"
- **THEN** cells for "bold" have the `bold` attribute set to `true`
- **AND** cells for "normal" have the `bold` attribute set to `false`

#### Scenario: Unknown escape sequences are ignored

- **WHEN** the shell outputs "text\\x1b[?25htext2"
- **THEN** the unknown sequence is ignored
- **AND** the grid contains "texttext2"

### Requirement: Dirty row tracking

The system SHALL track which rows have changed since the last snapshot read.

- After each VTE parse cycle, rows with any cell change SHALL be marked as dirty
- A snapshot request SHALL optionally accept a `full` boolean parameter
- When `full: true` (default), all rows SHALL be returned
- When `full: false`, only dirty rows since the last snapshot SHALL be returned
- After returning a `full: false` snapshot, the dirty row flags SHALL be reset
- If a resize occurs, all rows SHALL be marked dirty

#### Scenario: Incremental snapshot returns only changed rows

- **WHEN** a full snapshot is taken first
- **AND** new output changes only 3 rows
- **AND** a follow-up snapshot is requested with `full: false`
- **THEN** the response contains only the 3 changed rows
- **AND** the `damage` field lists the row indices that changed
