# WebSocket Connection

## Purpose

Ensure event bus WebSocket connection uniqueness and reliability in `teshi web` mode, preventing terminal I/O character duplication caused by multiple connections.

## Requirements

### Requirement: Event WebSocket Connection Uniqueness

The system SHALL ensure a single WebSocket connection to the event bus per page lifecycle, preventing duplicate event dispatch caused by multiple connections.

#### Scenario: Consecutive `ensureEventsSocket()` calls create one connection

- **WHEN** `ensureEventsSocket()` is called multiple times while the WebSocket is in `CONNECTING` state
- **THEN** only one WebSocket is created; subsequent calls reuse the existing connection

#### Scenario: Auto-reconnect on disconnect

- **WHEN** the current WebSocket connection closes (`onclose` fires and it is the current connection)
- **THEN** `eventsSocket` is set to null, and the next `ensureEventsSocket()` call creates a new connection

#### Scenario: Orphaned WebSocket close does not affect current connection

- **WHEN** a non-current WebSocket connection closes
- **THEN** the active `eventsSocket` reference is unaffected
