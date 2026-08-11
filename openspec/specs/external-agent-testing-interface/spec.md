# external-agent-testing-interface

## Purpose

Defines a stable machine-readable boundary through which external coding agents discover Teshi browser sessions and acquire verified Playwright locators without depending on Teshi's interactive UI.

## Requirements

### Requirement: Machine-readable browser locator operations
Teshi SHALL expose session discovery, lease management, tab discovery, page snapshot, locator resolution, locator verification, and optional evidence capture as machine-readable operations with documented schemas and failure semantics.

#### Scenario: Agent requests a locator successfully
- **WHEN** an external agent supplies a valid target, lease, and locator intent
- **THEN** Teshi SHALL return a structured success result containing the operation name, request identifier, browser target, page context, and locator result

#### Scenario: Operation fails
- **WHEN** an operation cannot complete
- **THEN** Teshi SHALL return a stable error code, actionable message, and non-sensitive recovery metadata and SHALL report failure status

### Requirement: CLI and MCP semantic parity
Teshi SHALL provide external-agent browser locator operations through its CLI and an optional local STDIO MCP server, and equivalent operations SHALL use the same validation, routing, lease, timeout, and result semantics.

#### Scenario: Same locator is requested through both adapters
- **WHEN** CLI and MCP callers request the same locator against the same page context
- **THEN** both adapters SHALL use the same candidate ranking and verification implementation

### Requirement: Explicit target and ownership inputs
Mutating and locator-acquisition operations SHALL accept an explicit browser session, tab target, and lease token and SHALL NOT depend on a UI shell's current selection.

#### Scenario: Desktop UI selects another tab
- **WHEN** an agent has explicitly targeted and leased a browser session and tab
- **THEN** unrelated UI-shell selection changes SHALL NOT reroute the agent's operation to another session

### Requirement: Agent-consumable session discovery
Session discovery SHALL return enough non-sensitive metadata for an agent or user to distinguish live browser instances and tabs and SHALL indicate disconnected, incompatible, and leased states.

#### Scenario: Agent sees several profiles
- **WHEN** multiple extension instances are live
- **THEN** discovery SHALL return their opaque identifiers, labels, browser versions, health, lease availability, and eligible tab metadata

### Requirement: Same-host browser execution
Browser extension operations SHALL execute through the broker on the host and user session that owns the browser profiles; a remote MCP caller SHALL NOT imply unauthenticated access to another host's browser.

#### Scenario: Local MCP server invokes a locator operation
- **WHEN** an agent connects to Teshi's local STDIO MCP server
- **THEN** the operation SHALL address only sessions registered with that local Teshi broker

### Requirement: Compatibility behavior for existing commands
Existing browser CLI commands SHALL continue to work without explicit session arguments only when Teshi can select exactly one eligible target.

#### Scenario: Existing user has one connected profile
- **WHEN** one eligible extension session and tab are connected and the user runs a legacy snapshot command
- **THEN** Teshi SHALL resolve that target and return behavior compatible with the existing command
