# winapp-preview-proxy Specification

## Purpose

Same-origin daemon transport that relays the loopback WinApp capture stream to
GPUI WASM clients while enforcing origin checks, sidecar privacy, and bounded
frame delivery.
## Requirements
### Requirement: Same-origin preview endpoint
The daemon SHALL expose browser and WinApp preview through `/ws/preview`, SHALL accept upgrades only from the exact trusted hosted Origin, and SHALL require a valid preview-channel first-message handshake before connecting to a sidecar or relaying output.

#### Scenario: LAN browser connects through daemon
- **WHEN** a browser loaded from the daemon origin upgrades `/api/v1/browser/stream` after WinApp mode starts
- **THEN** the daemon connects to its loopback sidecar and relays preview protocol messages to that browser

#### Scenario: Hosted browser connects through daemon
- **WHEN** the trusted hosted UI upgrades `/ws/preview` and authenticates a live hosted session compatible with the preview protocol
- **THEN** the daemon SHALL connect to the active loopback sidecar and relay supported preview protocol messages to that browser

#### Scenario: Cross-origin upgrade is rejected
- **WHEN** the preview endpoint receives an upgrade request whose Origin is not the exact configured trusted hosted Origin
- **THEN** the daemon SHALL reject the request before connecting to the sidecar

#### Scenario: Preview handshake is invalid
- **WHEN** an upgraded preview socket does not provide a valid first-message token/channel/protocol handshake
- **THEN** the daemon SHALL close it without attaching to a target or revealing preview data

### Requirement: Sidecar remains private
The daemon SHALL keep capture sidecars bound to loopback and SHALL NOT require the hosted browser to connect to or know a sidecar URL. The hosted client SHALL derive only the daemon `/ws/preview` endpoint from validated launch fragment data.

#### Scenario: Remote client selects endpoint
- **WHEN** the GPUI WASM client starts WinApp preview without a diagnostic endpoint override
- **THEN** it derives the daemon endpoint from validated launch state and uses `/ws/preview`

#### Scenario: Hosted client selects preview endpoint
- **WHEN** the GPUI WASM client starts browser or WinApp preview without a diagnostic endpoint override
- **THEN** it SHALL connect to `ws://127.0.0.1:<launch-port>/ws/preview`
- **AND** no daemon response or UI state SHALL expose the underlying sidecar URL

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

### Requirement: Preview session depends on an authenticated control session
A preview connection SHALL authenticate the same ephemeral hosted session and SHALL be accepted only while its control session is valid or within a documented bounded reconnect grace period.

#### Scenario: Preview token has no live control session
- **WHEN** a client authenticates `/ws/preview` with an otherwise valid token whose control session is absent and no reconnect grace applies
- **THEN** the daemon SHALL reject preview attachment and SHALL NOT connect to a sidecar
