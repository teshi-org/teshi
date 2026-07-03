# Terminal Exec

## Purpose

Write a command to the PTY and wait for it to complete, returning the final screen grid and exit indication. Provide a blocking command-execution workflow suitable for AI agent task automation.

## ADDED Requirements

### Requirement: Command execution with completion wait

The system SHALL accept a command string, write it to the PTY, and wait for the command to finish before returning.

- The endpoint SHALL accept a JSON request body with a `command` string and optional `timeout_ms` (default: 60000)
- The system SHALL write the command to the PTY followed by a newline (`\\n`)
- The system SHALL monitor the process state after writing
- "Command finished" SHALL be determined by detecting the next shell prompt appearance after the command output, with `state` transitioning from `running` back to `waiting_input` or `idle`
- If the timeout is reached before the command finishes, the system SHALL return the current snapshot with a `timed_out: true` flag
- The response SHALL include the terminal snapshot at the moment of completion or timeout
- The response SHALL include the elapsed time in milliseconds (`elapsed_ms`)

#### Scenario: Exec runs command and returns output

- **WHEN** a request is sent with `{"command": "echo hello"}`
- **THEN** the system writes "echo hello\\n" to the PTY
- **AND** waits until the next prompt is detected
- **AND** returns the screen grid showing "hello" in the output
- **AND** the response includes `"timed_out": false` and `"elapsed_ms": <duration>`

#### Scenario: Exec timeout returns partial output

- **WHEN** a request is sent with `{"command": "sleep 30", "timeout_ms": 5000}`
- **AND** the command is still running after 5 seconds
- **THEN** the system returns the current screen snapshot
- **AND** the response includes `"timed_out": true`

### Requirement: Exec on non-existent terminal

The system SHALL create a new PTY session when an exec request arrives but no terminal session is active.

- If no terminal has been spawned, the system SHALL call `spawn_terminal` with default dimensions (80×24) before writing the command
- Lazy spawn SHALL be transparent to the caller (no extra step required)
- If lazy spawn fails (e.g., no shell available), the response SHALL return an error with `"ok": false`

#### Scenario: First exec auto-starts terminal

- **WHEN** an exec request is the first terminal operation
- **AND** no terminal session exists
- **THEN** the system auto-spawns a shell
- **AND** then writes the command
- **AND** returns the result normally

### Requirement: Exec returns screen grid

The exec response SHALL include a snapshot of the terminal screen at the moment of completion or timeout.

- The `output` field SHALL contain the same structure as a snapshot response: `rows`, `cols`, `cells`, `cursor`, `state`
- The caller can use this output to determine the command's result without a separate snapshot call

#### Scenario: Exec response contains full snapshot

- **WHEN** an exec request completes
- **THEN** the response includes `"output": { "rows": 24, "cols": 80, "cells": [...], "cursor": [23, 3], "state": "waiting_input" }`
