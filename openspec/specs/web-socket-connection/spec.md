# WebSocket Connection

## Purpose

Ensure event bus WebSocket connection uniqueness and reliability in `teshi web` mode, preventing terminal I/O character duplication caused by multiple connections.
## Requirements
### Requirement: Event WebSocket Connection Uniqueness
The hosted GPUI WASM application SHALL maintain at most one current `/ws/control` connection per page lifecycle and hosted session, including while the connection is in `CONNECTING` or handshake state, so RPC responses and runtime events are not duplicated.

#### Scenario: Consecutive `ensureEventsSocket()` calls create one connection
- **WHEN** `ensureEventsSocket()` is called multiple times while the WebSocket is in `CONNECTING` state
- **THEN** only one WebSocket is created; subsequent calls reuse the existing connection

#### Scenario: Consecutive ensure calls occur during connection setup
- **WHEN** control-connection initialization is requested multiple times while the current socket is connecting or authenticating
- **THEN** only one WebSocket SHALL be created and subsequent callers SHALL reuse its pending result

#### Scenario: Current control connection disconnects
- **WHEN** the current authenticated control WebSocket closes
- **THEN** the active reference SHALL be cleared and any reconnect SHALL repeat `client_hello` before restoring safe subscriptions
- **AND** non-idempotent requests SHALL NOT be replayed automatically

#### Scenario: Auto-reconnect on disconnect
- **WHEN** the current WebSocket connection closes (`onclose` fires and it is the current connection)
- **THEN** the active connection reference is cleared, and the next connection repeats `client_hello`

#### Scenario: Orphaned WebSocket closes
- **WHEN** a non-current or superseded WebSocket emits a close callback
- **THEN** the active control connection reference and authenticated state SHALL remain unaffected

#### Scenario: Orphaned WebSocket close does not affect current connection
- **WHEN** a non-current WebSocket connection closes
- **THEN** the active `eventsSocket` reference is unaffected
