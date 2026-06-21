# Exploration UI

Terminal UI that provides live visibility into the agent's exploration
session, including browser state display, action timeline, and manual
override controls.

## Purpose

Enable the user to observe and interact with the exploration session in
real time — see the current browser state, review the chronological action
trace, pause/resume the agent, and manually take over when the agent gets
stuck.

## ADDED Requirements

### Requirement: Live agent browser state display
The TUI SHALL display the current browser state during exploration, showing which page the agent is viewing and what elements are visible.

### Requirement: Step trace with snapshots
The TUI SHALL show a chronological timeline of agent actions with the resulting page state after each action.

### Requirement: Manual pause and resume
The TUI SHALL provide controls to pause and resume the exploration session.

### Requirement: Manual override controls
The TUI SHALL allow the user to manually take over and perform actions when the agent gets stuck.
