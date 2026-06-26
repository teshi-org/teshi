# Terminal Loop Detection

## Purpose

Prevent runaway terminal output from freezing the application by detecting output loops in the PTY reader, killing the offending shell, and safely respawning with exponential backoff.

## Requirements

### Requirement: PTY output rate monitoring

The terminal PTY reader SHALL monitor the rate of output from the shell process.

- The reader SHALL count the number of bytes read from the PTY in each 100ms sliding window
- If the output rate exceeds 1MB/s for any consecutive 2 windows (200ms), the system SHALL consider this a loop condition
- The system SHALL emit a `terminal-loop-detected` event when a loop condition is detected
- Upon loop detection, the system SHALL kill the shell process and wait before allowing respawn
- The respawn delay SHALL follow exponential backoff: 1s, 2s, 4s, 8s, 16s, capped at 30s
- The backoff counter SHALL reset to 1s after a shell has been running normally for 30+ seconds

#### Scenario: Loop detection triggers shell reset

- **WHEN** the shell produces more than 1MB/s of output for 200ms
- **THEN** the system kills the shell
- **AND** the frontend shows a "Terminal output loop detected, restarting..." message
- **AND** the auto-respawn uses exponential backoff

#### Scenario: Normal output does not trigger loop detection

- **WHEN** the shell produces normal command output (e.g., < 100KB/s)
- **THEN** the output is forwarded normally
- **AND** no loop detection is triggered

### Requirement: Frontend respawn debounce

The frontend SHALL prevent rapid shell respawn loops.

- After a shell exit, the frontend SHALL wait at least 1 second before auto-respawning
- If the shell exits 3 or more times within 10 seconds, auto-respawn SHALL stop
- The user can always manually restart the shell via the "Restart shell" button regardless of debounce state

#### Scenario: Rapid shell exits stop auto-respawn

- **WHEN** the shell exits 3 times within 10 seconds
- **THEN** auto-respawn stops
- **AND** a message "Auto-restart paused due to repeated exits" is shown
- **AND** the "Restart shell" button remains functional
