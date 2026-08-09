## ADDED Requirements

### Requirement: Same-origin preview endpoint
The daemon SHALL expose the active WinApp preview through a WebSocket endpoint on the daemon origin and SHALL reject cross-origin upgrade requests.

#### Scenario: LAN browser connects through daemon
- **WHEN** a browser loaded from the daemon origin upgrades `/api/v1/browser/stream` after WinApp mode starts
- **THEN** the daemon connects to its loopback sidecar and relays preview protocol messages to that browser

#### Scenario: Cross-origin upgrade is rejected
- **WHEN** the preview endpoint receives an upgrade request whose `Origin` does not match the request host
- **THEN** the daemon rejects the request before connecting to the sidecar

### Requirement: Sidecar remains private
The daemon SHALL keep the capture sidecar bound to loopback and SHALL NOT require the browser to connect to or know the sidecar's loopback URL.

#### Scenario: Remote client selects endpoint
- **WHEN** the GPUI WASM client starts WinApp preview without a diagnostic endpoint override
- **THEN** it derives the WebSocket scheme and authority from the current page and uses `/api/v1/browser/stream`

### Requirement: Bounded frame relay
The daemon SHALL prevent a slow preview client from creating an unbounded frame queue or indefinitely blocking upstream frame reads.

#### Scenario: Viewer cannot keep up
- **WHEN** newer frames arrive before the browser has consumed the prior buffered frame
- **THEN** the buffered frame is replaced with the most recent frame while control and error messages remain independently bounded

### Requirement: Narrow capture command surface
The preview proxy SHALL initiate the configured prototype window attachment itself and SHALL NOT forward arbitrary browser text commands to the capture sidecar.

#### Scenario: Stream is upgraded
- **WHEN** the daemon establishes the sidecar WebSocket for a preview client
- **THEN** it requests attachment to the configured target process and only relays sidecar output to the client
