## ADDED Requirements

### Requirement: Rust broker runtime
Teshi SHALL provide a Rust broker server and SHALL use it as the only production implementation for Chrome extension mode after migration acceptance. The broker SHALL be started by Teshi as a detached internal process, SHALL be shared by all Teshi surfaces for the same OS user, and SHALL NOT require a separately managed user service.

#### Scenario: Two Teshi surfaces start together
- **WHEN** CLI, Daemon, or Desktop processes concurrently request the Chrome broker
- **THEN** exactly one compatible per-user broker SHALL be active and all callers SHALL attach to its advertised instance

#### Scenario: An unrelated process occupies discovery port
- **WHEN** the fixed discovery port is occupied by a service that does not satisfy the Teshi protocol and feature preflight
- **THEN** Teshi SHALL report the conflict without terminating or sending commands to that process

#### Scenario: Broker crashes and restarts
- **WHEN** the broker process exits or crashes
- **THEN** new callers SHALL detect the stale process identity, start a compatible broker under the startup lock, and reject late messages from the previous broker generation

### Requirement: Compatible bounded transport
The Rust broker SHALL preserve HTTP discovery on port 17373, a dynamically selected WebSocket port, existing protocol-v1 message names and fields, TSH1 binary preview frames, and legacy implicit targeting rules. It SHALL bound connections, queue lengths, request bodies, WebSocket text/binary messages, and operation timeouts.

#### Scenario: Existing extension protocol v1 connects
- **WHEN** a supported protocol-v1 extension registers and opens its preview stream
- **THEN** the Rust broker SHALL return compatible heartbeat responses and accept correlated direct commands, binary preview frames, and acknowledged Network batches

#### Scenario: Oversized or malformed message arrives
- **WHEN** a client sends an oversized, malformed, or unknown privileged operation
- **THEN** the broker SHALL reject it within configured resource limits without allocating unbounded state or mutating a browser target

### Requirement: Exactly-once request lifecycle
Each request SHALL be scoped to one complete browser target and SHALL transition at most once from queued or sent to completed, failed, cancelled, or expired. Disconnects, lease expiry, mismatched responses, and late responses SHALL NOT route work to another Profile or turn a failed operation into success.

#### Scenario: Target disconnects with pending work
- **WHEN** an extension connection closes while a request is pending
- **THEN** the broker SHALL fail that request with a stable disconnect code, release its owned resources as specified, and leave unrelated sessions unchanged

#### Scenario: A response arrives after cancellation
- **WHEN** a response arrives after its request timed out, was cancelled, or its broker generation changed
- **THEN** the broker SHALL reject or quarantine it and SHALL NOT complete another request

#### Scenario: A mutating lease expires before dispatch
- **WHEN** a queued mutation reaches dispatch after its lease has expired
- **THEN** the broker SHALL fail the operation before sending it to the extension

### Requirement: Mode-specific runtimes stay isolated
Chrome extension mode SHALL use the Rust broker; Embedded mode SHALL retain its existing Python Playwright sidecar; WinApp SHALL retain its existing managed runtime. Starting Chrome SHALL NOT perform Python, pip, uv, virtual-environment, or Playwright checks.

#### Scenario: Embedded mode starts
- **WHEN** a user explicitly selects Embedded browser mode
- **THEN** Teshi SHALL continue to start the existing Playwright backend and report its own endpoint and errors

#### Scenario: WinApp starts
- **WHEN** a user explicitly starts WinApp automation
- **THEN** Teshi SHALL continue to use the existing managed WinApp runtime without starting the Chrome broker as its executor
