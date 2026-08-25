## ADDED Requirements

### Requirement: Machine-readable filtered network capture
Teshi SHALL expose start, list, detail, clear, and stop operations for an explicit leased target, and start SHALL require one or more exact hostnames while optionally enabling bounded request-body retention.

#### Scenario: Agent captures one API hostname
- **WHEN** an agent starts capture with an explicit target, lease, hostname, and request-body option
- **THEN** the operation SHALL return a capture identifier, normalized filters, limits, and delivery diagnostics suitable for subsequent list, detail, clear, and stop commands

### Requirement: Capture compatibility failure
Enhanced network capture SHALL verify advertised extension and broker capabilities before dispatch and SHALL return a stable error instead of degrading to broader capture.

#### Scenario: Network batch transport is unavailable
- **WHEN** the selected session lacks acknowledged network-batch transport
- **THEN** the CLI SHALL fail non-zero with actionable compatibility metadata and SHALL perform no capture
