## MODIFIED Requirements

### Requirement: Machine-readable browser locator operations
Teshi SHALL expose session and target discovery, lease management, page snapshots, locator resolution and verification, safe browser-control primitives, bounded observability/artifact operations, and optional evidence capture as machine-readable CLI operations with documented schemas and failure semantics. Privileged P2 operations SHALL be advertised separately and SHALL require their capability contracts.

#### Scenario: Agent requests a locator successfully
- **WHEN** an external agent supplies a valid target, lease, and locator intent
- **THEN** Teshi SHALL return a structured success result containing the operation name, request identifier, browser target, page context, and locator result

#### Scenario: Agent executes a safe control operation
- **WHEN** an external agent supplies a valid target, lease, current reference or structured candidate, action, and optional typed wait
- **THEN** Teshi SHALL return separately structured action and wait results correlated to the same request

#### Scenario: Operation fails
- **WHEN** an operation cannot complete
- **THEN** Teshi SHALL return a stable error code, actionable message, and non-sensitive recovery metadata and SHALL report failure status

#### Scenario: Artifact operation fails across protocol versions
- **WHEN** an artifact operation returns either the current or immediately preceding wire spelling
- **THEN** Teshi SHALL normalize it to the canonical `browser_artifact_failure` code

## ADDED Requirements

### Requirement: CLI and MCP exposure policy
Teshi SHALL implement browser operations once in the shared typed layer while allowing adapters to expose a least-privilege subset; all locator operations SHALL retain CLI/MCP semantic parity, safe control tools MAY require explicit MCP enablement, and privileged tools SHALL be absent by default.

#### Scenario: MCP control tools are disabled
- **WHEN** the local MCP server starts without safe-control or privileged allowlists
- **THEN** locator operations SHALL remain available and mutation or P2 tools SHALL not be advertised

### Requirement: Machine-readable capability discovery
Teshi SHALL return per-backend protocol features, supported actions, artifact capabilities, optional browser permissions, and effective policy availability without returning secret grants.

#### Scenario: Agent plans a pointer click
- **WHEN** the selected session does not advertise pointer input
- **THEN** the agent SHALL be able to detect the missing capability before acquiring or executing the action
