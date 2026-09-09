## MODIFIED Requirements

### Requirement: Event WebSocket Connection Uniqueness
The hosted GPUI WASM application SHALL maintain at most one current `/ws/control` connection per page lifecycle and hosted session, including while the connection is in `CONNECTING` or handshake state, so RPC responses and runtime events are not duplicated.

#### Scenario: Consecutive ensure calls occur during connection setup
- **WHEN** control-connection initialization is requested multiple times while the current socket is connecting or authenticating
- **THEN** only one WebSocket SHALL be created and subsequent callers SHALL reuse its pending result

#### Scenario: Current control connection disconnects
- **WHEN** the current authenticated control WebSocket closes
- **THEN** the active reference SHALL be cleared and any reconnect SHALL repeat `client_hello` before restoring safe subscriptions
- **AND** non-idempotent requests SHALL NOT be replayed automatically

#### Scenario: Orphaned WebSocket closes
- **WHEN** a non-current or superseded WebSocket emits a close callback
- **THEN** the active control connection reference and authenticated state SHALL remain unaffected
