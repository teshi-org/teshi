## ADDED Requirements

### Requirement: ACP sessions expose live Teshi tools

Teshi SHALL attach a conversation-specific STDIO MCP server to ACP sessions and expose only tools allowed by the selected profile. Calls MUST execute against the owning live TUI host and return actual results or errors.

#### Scenario: Requirements reach the live review workflow
- **WHEN** an ACP agent calls submit_requirements and propose_test_points
- **THEN** the current TUI stores the requirements and shows the proposed points for human review

#### Scenario: Restricted profile
- **WHEN** a tool is not allowed by the profile
- **THEN** discovery omits it and execution rejects it

#### Scenario: Profile tool permissions change
- **WHEN** the user selects a profile with different tool permissions
- **THEN** ACP reconnects with the new catalog and newly allowed tools become discoverable

### Requirement: Approval remains authoritative

ACP tool access MUST preserve mandatory human test-point review and file approval. Auto and Bypass permissions MUST NOT approve test points.

#### Scenario: Planning before human review
- **WHEN** the ACP agent requests planning before test points are approved
- **THEN** the existing host gate rejects the request

#### Scenario: Manual file change
- **WHEN** an ACP tool queues a file change in Manual mode
- **THEN** the tool result remains pending until the user accepts or rejects the change

#### Scenario: Manual acceptance fails
- **WHEN** accepting a queued change fails to write the file
- **THEN** the caller receives a tool error and the failed proposal is cleared

### Requirement: Bridge isolation and cleanup

The bridge MUST bind only to loopback, authenticate requests with an ephemeral token, and close when its backend stops. Cancelled or disconnected tool requests MUST NOT mutate the project.

#### Scenario: Invalid token
- **WHEN** a connection provides an incorrect token
- **THEN** it cannot discover or execute tools

#### Scenario: Backend closes with a pending call
- **WHEN** the owning backend is closed
- **THEN** the listener closes and pending calls terminate

#### Scenario: STDIO cancellation or disconnect while awaiting approval
- **WHEN** the MCP client cancels a pending call or closes STDIO
- **THEN** the proxy closes the corresponding bridge connection and the change cannot be accepted
