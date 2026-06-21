# Exploration Loop

Orchestrates the agent's step-by-step traversal of a Gherkin scenario,
managing state, trace recording, sandboxing, and termination conditions.

## Purpose

Drive the LLM agent through a Gherkin scenario one step at a time: observe
page state via `browser_snapshot`, decide an action, execute it, record the
trace, and continue until the scenario completes or a termination condition
is met.

## ADDED Requirements

### Requirement: Gherkin-scenario-driven exploration
The system SHALL read a Gherkin scenario context from the active buffer (feature path, step lines, step texts, and Given/When/Then keywords) and drive the agent through the scenario step by step.

### Requirement: Step-level exploration flow
For each step in the scenario, the agent SHALL observe the page state via `browser_snapshot`, decide an action, execute it, and move to the next step.

### Requirement: Exploration trace buffer
The system SHALL maintain an ordered trace of actions with step_line, timestamp, ref, action type, arguments, and resulting snapshot.

### Requirement: Step counter with configurable limit
The system SHALL track the number of steps taken and terminate exploration when a configurable limit (default 15) is exceeded, marking the trace as incomplete.

#### Scenario: Step limit exceeded
- **WHEN** the agent exceeds `max_steps` actions
- **THEN** exploration SHALL terminate with "step limit exceeded" reason
- **AND** the trace SHALL be saved as incomplete

### Requirement: URL whitelist sandbox
The system SHALL check the page URL against allowed patterns after each navigation. On violation, the system SHALL navigate back and log a boundary violation.

### Requirement: reset_environment tool
The system SHALL provide a `reset_environment` tool that restores application state to a clean baseline before exploration starts.

### Requirement: Loop termination conditions
The exploration loop SHALL handle the following termination conditions: all steps bound, step limit exceeded, URL boundary violation, unrecoverable error, and user cancel.

### Requirement: Switchable agent mode
The system SHALL support switching between chat mode and explore mode, properly pausing and resuming LLM streaming when switching modes.
