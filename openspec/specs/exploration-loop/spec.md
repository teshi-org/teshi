# Exploration Loop

Orchestrates the agent's step-by-step traversal of a Gherkin scenario,
managing state, trace recording, sandboxing, and termination conditions.

## Purpose

Drive the LLM agent through a Gherkin scenario one step at a time: observe
page state via `browser_snapshot`, decide an action, execute it, record the
trace, and continue until the scenario completes or a termination condition
is met.

## Requirements

### Requirement: Gherkin-scenario-driven exploration

The system SHALL read a Gherkin scenario context from the active buffer (feature path, step lines, step texts, and Given/When/Then keywords) and drive the agent through the scenario step by step.

#### Scenario: Exploration starts from the active scenario

- **WHEN** exploration starts with a Gherkin scenario selected in the active buffer
- **THEN** the agent context SHALL include its feature path, step lines, step texts, and keywords

### Requirement: Step-level exploration flow

For each step in the scenario, the agent SHALL observe the page state via `browser_snapshot`, decide an action, execute it, and move to the next step.

#### Scenario: Agent processes a scenario step

- **WHEN** the agent begins processing a scenario step
- **THEN** it SHALL observe the page before choosing and executing an action
- **AND** it SHALL advance only after recording the action result

### Requirement: Exploration trace buffer

The system SHALL maintain an ordered trace of actions with step_line, timestamp, ref, action type, arguments, and resulting snapshot.

#### Scenario: Completed action is recorded

- **WHEN** an exploration action completes
- **THEN** the trace SHALL append its step line, timestamp, reference, action type, arguments, and resulting snapshot in execution order

### Requirement: Step counter with configurable limit
The system SHALL track the number of steps taken and terminate exploration when a configurable limit (default 15) is exceeded, marking the trace as incomplete.

#### Scenario: Step limit exceeded
- **WHEN** the agent exceeds `max_steps` actions
- **THEN** exploration SHALL terminate with "step limit exceeded" reason
- **AND** the trace SHALL be saved as incomplete

### Requirement: URL whitelist sandbox

The system SHALL check the page URL against allowed patterns after each navigation. On violation, the system SHALL navigate back and log a boundary violation.

#### Scenario: Navigation leaves the allowed boundary

- **WHEN** a navigation reaches a URL that matches no allowed pattern
- **THEN** the system SHALL navigate back and record a boundary violation

### Requirement: reset_environment tool

The system SHALL provide a `reset_environment` tool that restores application state to a clean baseline before exploration starts.

#### Scenario: Environment is reset before exploration

- **WHEN** an exploration run requests a clean baseline
- **THEN** `reset_environment` SHALL restore the application state before the first scenario action

### Requirement: Loop termination conditions

The exploration loop SHALL handle the following termination conditions: all steps bound, step limit exceeded, URL boundary violation, unrecoverable error, and user cancel.

#### Scenario: User cancels exploration

- **WHEN** the user cancels an active exploration run
- **THEN** the loop SHALL stop, preserve the trace, and record user cancellation as the termination reason

### Requirement: Switchable agent mode

The system SHALL support switching between chat mode and explore mode, properly pausing and resuming LLM streaming when switching modes.

#### Scenario: User switches away from explore mode

- **WHEN** the user switches from explore mode to chat mode during a run
- **THEN** exploration streaming SHALL pause without discarding the current run state
- **AND** it SHALL be resumable when the user returns to explore mode
