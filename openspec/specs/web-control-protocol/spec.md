# web-control-protocol Specification

## Purpose
TBD - created by archiving change decouple-nightly-cli-and-hosted-web-ui. Update Purpose after archive.
## Requirements
### Requirement: Control channel carries correlated RPC
After a successful handshake, `/ws/control` SHALL accept versioned namespaced requests with an opaque client correlation id and SHALL return exactly one terminal response with the same id for every accepted request.

#### Scenario: RPC succeeds
- **WHEN** an authorized client sends a valid request envelope
- **THEN** the daemon SHALL return a success response with the same correlation id and the method result

#### Scenario: RPC fails
- **WHEN** a request is malformed, unauthorized, unknown, or fails in the domain handler
- **THEN** the daemon SHALL return a structured error response with the same correlation id when one was validly supplied
- **AND** one failed request SHALL NOT terminate unrelated in-flight requests

### Requirement: Control channel carries ordered domain events
The authenticated control connection SHALL carry runtime events, including Terminal and Agent activity, in a versioned event envelope and SHALL preserve their emission order within that connection.

#### Scenario: Client subscribes and runtime emits events
- **WHEN** an authenticated UI has established its control session and the runtime emits subscribed events
- **THEN** the daemon SHALL deliver them on that same control WebSocket without creating an event-only socket

### Requirement: Control backpressure is bounded
The control protocol SHALL use bounded buffering and explicit overflow behavior so bursty Terminal, Agent, or runtime events cannot create unbounded memory growth or indefinitely starve RPC responses.

#### Scenario: Client cannot consume a burst
- **WHEN** event production exceeds the connection's bounded delivery capacity
- **THEN** the daemon SHALL apply the documented event-specific backpressure, coalescing, or disconnect policy
- **AND** it SHALL NOT allocate an unbounded queue

### Requirement: Preview traffic is excluded from control
Screenshot frames and other high-volume browser/WinApp preview payloads SHALL travel only over `/ws/preview` and SHALL NOT share the control connection's delivery queue.

#### Scenario: Preview consumer is slow
- **WHEN** the preview client stalls while the UI issues a control RPC
- **THEN** preview buffering SHALL NOT block the control response or control-event delivery

### Requirement: Hosted UI operations have complete WebSocket coverage
Phase one SHALL maintain a checked inventory mapping every business operation used by the hosted GPUI WASM UI across LLM configuration, Browser, Project, filesystem, Gherkin/BDD, run/exchange, locator/steps, Terminal, Agent, and daemon lifecycle to a control RPC/event or preview message.

#### Scenario: Migration coverage is audited
- **WHEN** the hosted UI is accepted for production deployment
- **THEN** its normal business workflows SHALL make no `/api/v1/*` request
- **AND** every inventoried operation SHALL have protocol and integration coverage

### Requirement: Legacy REST remains during phase one
Adding and adopting the WebSocket protocols SHALL NOT delete the existing `/api/v1/*` REST routes in this change.

#### Scenario: WebSocket migration completes
- **WHEN** the hosted UI passes its WebSocket acceptance suite
- **THEN** the existing REST route registration and existing REST regression tests SHALL still be present
- **AND** REST cleanup SHALL require a separate reviewed change
