# Exploration UI

Terminal UI that provides live visibility into the agent's exploration
session, including browser state display, action timeline, and manual
override controls.

## Purpose

Enable the user to observe and interact with the exploration session in
real time — see the current browser state, review the chronological action
trace, pause/resume the agent, and manually take over when the agent gets
stuck.

## Requirements

### Requirement: Live agent browser state display

The TUI SHALL display the current browser state during exploration, showing which page the agent is viewing and what elements are visible.

#### Scenario: Browser state changes during exploration

- **WHEN** the exploration session receives a new browser snapshot
- **THEN** the TUI SHALL show the current page and its visible elements

### Requirement: Step trace with snapshots

The TUI SHALL show a chronological timeline of agent actions with the resulting page state after each action.

#### Scenario: Agent action completes

- **WHEN** an agent action and its resulting snapshot are recorded
- **THEN** the TUI SHALL append them to the chronological exploration timeline

### Requirement: Manual pause and resume

The TUI SHALL provide controls to pause and resume the exploration session.

#### Scenario: User pauses and resumes exploration

- **WHEN** the user activates pause and later activates resume
- **THEN** the TUI SHALL suspend agent progression and then continue the same exploration session

### Requirement: Manual override controls

The TUI SHALL allow the user to manually take over and perform actions when the agent gets stuck.

#### Scenario: User takes over an active session

- **WHEN** the user activates manual override during exploration
- **THEN** the TUI SHALL stop autonomous actions and allow the user to operate the current browser state
