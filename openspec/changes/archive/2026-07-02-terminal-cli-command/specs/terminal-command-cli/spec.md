# Terminal Command CLI

## Purpose

Register `teshi terminal` as a CLI subcommand in the teshi binary, following the same pattern as `teshi browser` and `teshi winapp`, enabling AI agents and developers to control the terminal via shell commands with JSON output.

## ADDED Requirements

### Requirement: CLI subcommand registration

The `teshi` CLI SHALL register a `terminal` subcommand with the following sub-actions:

- `serve-embedded` — Start the terminal sidecar (foreground, Ctrl+C to stop)
- `snapshot` — Read the current terminal screen grid
- `status` — Query terminal process state
- `exec <command>` — Execute a command and wait for completion
- `send <text>` — Write text to the terminal
- `resize <cols> <rows>` — Resize the terminal
- `kill` — Kill the current terminal session

All subcommands SHALL follow the existing `BrowserCommand`/`WinAppCommand` pattern: each is a clap `#[derive(Subcommand)]` enum variant with its own args struct.

#### Scenario: CLI help shows terminal subcommands

- **WHEN** the user runs `teshi terminal --help`
- **THEN** the output lists all sub-actions: serve-embedded, snapshot, status, exec, send, resize, kill

### Requirement: JSON output to stdout

Every `teshi terminal` subcommand SHALL print its result as a JSON object to stdout on success.

- The JSON SHALL be pretty-printed with `serde_json::to_string_pretty`
- On failure, the command SHALL exit with a non-zero exit code
- Error messages SHALL be written to stderr (via `anyhow::bail` or `eprintln!`)
- The JSON response SHALL include an `ok` boolean field
- This SHALL use the same `print_json_response` / `ensure_ok` pattern as browser/winapp

#### Scenario: Successful snapshot returns JSON

- **WHEN** `teshi terminal snapshot` succeeds
- **THEN** stdout receives a JSON object with `"ok": true` and the grid data
- **AND** the exit code is 0

#### Scenario: Failed command returns error

- **WHEN** `teshi terminal snapshot` fails (e.g., no terminal sidecar running)
- **THEN** stderr receives the error message
- **AND** the exit code is non-zero

### Requirement: Sidecar endpoint discovery via cdp-endpoint.json

The CLI SHALL discover the sidecar's WebSocket address by reading `.teshi/cdp-endpoint.json` from the project root, exactly like browser/winapp.

- The system SHALL read `.teshi/cdp-endpoint.json` and extract the `ws_url` field
- The `mode` field SHALL be validated — if present and not `"terminal"`, the CLI SHALL exit with error indicating mode mismatch
- The extracted `ws_url` SHALL be used directly as the WebSocket URL for `send_sidecar_command_with_timeout`
- If the file does not exist or cannot be parsed, the CLI SHALL exit with an error message guiding the user to run `teshi terminal serve-embedded` first

#### Scenario: Endpoint found and valid

- **WHEN** `.teshi/cdp-endpoint.json` exists with `ws_url: "ws://127.0.0.1:54321"` and `mode: "terminal"`
- **THEN** the CLI connects to `ws://127.0.0.1:54321` via WebSocket
- **AND** sends the command as a JSON frame
- **AND** returns the sidecar's response

#### Scenario: Endpoint not found

- **WHEN** `.teshi/cdp-endpoint.json` does not exist
- **THEN** the CLI prints: "terminal sidecar not found; run `teshi terminal serve-embedded` first"
- **AND** exits with code 1

#### Scenario: Mode mismatch

- **WHEN** `.teshi/cdp-endpoint.json` exists but `mode` is `"browser"` instead of `"terminal"`
- **THEN** the CLI prints: "expected mode 'terminal' but found 'browser'; start the terminal sidecar"
- **AND** exits with code 1

### Requirement: Sidecar writes cdp-endpoint.json on startup

The terminal sidecar SHALL write `.teshi/cdp-endpoint.json` with `mode: "terminal"` when it starts listening.

- The file SHALL be written to the project root's `.teshi/` directory
- The `ws_url` SHALL be `ws://127.0.0.1:<port>` where `<port>` is the randomly assigned listen port
- The `mode` field SHALL be `"terminal"`
- The file SHALL be written after the WebSocket server starts successfully
- If the file already exists from another mode, the sidecar SHALL overwrite it

#### Scenario: Sidecar writes endpoint on startup

- **WHEN** the terminal sidecar starts
- **AND** binds to a TCP port (e.g., 54321)
- **THEN** it writes `.teshi/cdp-endpoint.json` with `ws_url: "ws://127.0.0.1:54321"` and `mode: "terminal"`
