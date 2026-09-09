## MODIFIED Requirements

### Requirement: Same-origin preview endpoint
The daemon SHALL expose browser and WinApp preview through `/ws/preview`, SHALL accept upgrades only from the exact trusted hosted Origin, and SHALL require a valid preview-channel first-message handshake before connecting to a sidecar or relaying output.

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

#### Scenario: Hosted client selects preview endpoint
- **WHEN** the GPUI WASM client starts browser or WinApp preview without a diagnostic endpoint override
- **THEN** it SHALL connect to `ws://127.0.0.1:<launch-port>/ws/preview`
- **AND** no daemon response or UI state SHALL expose the underlying sidecar URL

## ADDED Requirements

### Requirement: Preview session depends on an authenticated control session
A preview connection SHALL authenticate the same ephemeral hosted session and SHALL be accepted only while its control session is valid or within a documented bounded reconnect grace period.

#### Scenario: Preview token has no live control session
- **WHEN** a client authenticates `/ws/preview` with an otherwise valid token whose control session is absent and no reconnect grace applies
- **THEN** the daemon SHALL reject preview attachment and SHALL NOT connect to a sidecar
