## ADDED Requirements

### Requirement: Phased browser-control compatibility declaration
The distributed browser-testing package SHALL declare P0, P1, and P2 protocol feature compatibility independently for the CLI, broker, extension, backend, Skill, and optional MCP exposure.

#### Scenario: Installed extension supports only P0
- **WHEN** a user requests a P1 operation with a P0-only extension
- **THEN** packaged guidance and machine-readable compatibility data SHALL identify the missing feature and compatible upgrade without disabling P0

### Requirement: Standalone CLI broker guidance
The distribution SHALL document and validate CLI-owned Chrome broker startup, reuse, shutdown, Desktop coexistence, and recovery without requiring source-checkout paths.

#### Scenario: User invokes browser sessions after installation
- **WHEN** no compatible broker is running
- **THEN** the installed CLI SHALL start its bundled broker resources and return actionable extension setup or session discovery results

### Requirement: Privileged capability policy guidance
The distribution SHALL document P2's default-deny policy, grant lifetime, optional browser permissions, MCP exclusions, audit location, redaction, and revocation separately from the safe P0/P1 workflow.

#### Scenario: User installs the browser-testing package
- **WHEN** installation completes without further privileged configuration
- **THEN** no JavaScript, raw-CDP, Cookie, content-setting, or extension-management grant SHALL be active

### Requirement: Packaged operational Skill coverage
The browser-testing package SHALL include agent guidance for target discovery, lease lifecycle, reference refresh, structured execution, typed waits, artifacts, diagnostics, and privileged capability boundaries.

#### Scenario: Agent encounters a stale reference
- **WHEN** the packaged Skill receives `stale_element_reference`
- **THEN** it SHALL instruct the agent to take a new snapshot and SHALL prohibit silent retargeting or mutation retry
