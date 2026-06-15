## ADDED Requirements

### Requirement: Live agent browser state display
The TUI SHALL display a panel showing the current state of the agent's browser exploration, including current URL, step count, last action, and navigation status.

#### Scenario: Panel shows current URL
- **WHEN** the exploration loop is active
- **THEN** the UI panel SHALL display the current browser URL updated in real time

#### Scenario: Panel shows step progress
- **WHEN** the exploration loop is active
- **THEN** the UI panel SHALL display the current step count and the maximum step limit

### Requirement: Trace replay
The TUI SHALL allow the user to review a completed exploration trace step by step, showing each action with its context and resulting page snapshot.

#### Scenario: User replays trace
- **WHEN** an exploration completes successfully
- **THEN** the user SHALL be able to step through each trace action in the UI

### Requirement: Manual override controls
The TUI SHALL provide pause, resume, and cancel controls for an active exploration, as well as a manual intervention mode where the user can assume control of the browser.

#### Scenario: User pauses exploration
- **WHEN** the user presses the pause key during active exploration
- **THEN** the exploration loop SHALL pause and wait for user input before continuing

#### Scenario: User cancels exploration
- **WHEN** the user presses the cancel key during active exploration
- **THEN** the exploration loop SHALL terminate and discard the partial trace
