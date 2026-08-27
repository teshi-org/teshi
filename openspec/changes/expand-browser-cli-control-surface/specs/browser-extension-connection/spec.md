## ADDED Requirements

### Requirement: CLI-startable broker lifecycle
The browser extension connection SHALL support a compatible loopback broker started by the CLI, Desktop, or an existing user-session process without depending on which shell initiated it.

#### Scenario: Desktop is closed
- **WHEN** the CLI starts the compatible broker and the extension reconnects
- **THEN** registered profiles SHALL expose the same identity, health, lease, and operation semantics as a Desktop-initiated connection

### Requirement: Negotiated phased feature support
Extension registration SHALL advertise supported P0, P1, and P2 protocol features independently so the broker can reject an unsupported operation without marking all session functions incompatible.

#### Scenario: P0 extension lacks P1 screenshot support
- **WHEN** the session is otherwise compatible and a caller requests a P1 screenshot
- **THEN** Teshi SHALL return the missing feature and required extension version while leaving P0 operations available

### Requirement: Optional privileged permissions
The extension SHALL declare Cookie, content-setting, and extension-management permissions as optional and SHALL request each only through a user-visible extension gesture.

#### Scenario: User revokes an optional permission
- **WHEN** an optional permission is revoked while the session remains connected
- **THEN** feature discovery SHALL update and affected P2 operations SHALL fail closed without disconnecting safe P0 operations

### Requirement: Low-latency correlated command transport
The extension and broker SHALL provide a negotiated correlated command path suitable for interactive control while retaining heartbeat-based liveness and bounded fallback behavior.

#### Scenario: Direct command channel disconnects
- **WHEN** the negotiated interactive command channel is unavailable but heartbeats remain healthy
- **THEN** Teshi SHALL either use the documented bounded fallback or return a transport-specific unavailable error without duplicating a mutating request

### Requirement: Authenticated loopback ingress
The broker SHALL authenticate every WebSocket connection and mutating HTTP fallback request with an unpredictable broker-start credential and SHALL reject ordinary browser-page origins.

#### Scenario: Web page attempts to impersonate the extension
- **WHEN** a browser page connects to a loopback command route or posts a fabricated response without the broker credential
- **THEN** the broker SHALL reject the request before registration, confirmation, command dispatch, or response correlation
