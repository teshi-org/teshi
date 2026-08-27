# TUI Requirements and Test-Points Authoring

## Purpose

Requirement documents and test points are durable, source-controlled project artifacts. The TUI provides Requirements and Test Points tabs for authoring, arbitrary text-range traceability with resilient re-anchoring, and mandatory human review before test points can drive Gherkin scenario planning.

## Requirements

### Requirement: Requirement documents are durable project artifacts

The system SHALL store requirement documents as Markdown files under the project's requirement root and SHALL maintain stable document identifiers independently of display titles. The TUI SHALL restore the requirement hierarchy and current document content after restart.

#### Scenario: User creates a requirement document

- **WHEN** the user creates a requirement document in the Requirements tab
- **THEN** the system SHALL persist the Markdown document and its stable identity under the project requirement root
- **AND** the document SHALL appear in the Requirements tree

#### Scenario: TUI restarts

- **WHEN** a project containing requirement artifacts is reopened
- **THEN** the TUI SHALL reconstruct the same requirement hierarchy and document identities from disk

#### Scenario: An indexed document is missing

- **WHEN** the requirement index references a Markdown file that no longer exists
- **THEN** the TUI SHALL report the missing document and SHALL NOT silently assign its identity to another file

### Requirement: The Requirements tab presents hierarchy, source, and linked test points

The TUI SHALL provide a Requirements tab with a requirement tree in the left pane, the selected requirement's Markdown content in the center pane, and linked test points in the right pane.

#### Scenario: User selects a requirement document

- **WHEN** the user selects a document in the requirement tree
- **THEN** the center pane SHALL display its current Markdown content
- **AND** the right pane SHALL display all test points linked to that document

#### Scenario: User selects a text range

- **WHEN** the user selects a non-empty range in the requirement content
- **THEN** the right pane SHALL filter to test points whose resolved anchors overlap the selection

#### Scenario: User selects a linked test point

- **WHEN** the user selects a test point in the right pane
- **THEN** the center pane SHALL highlight every resolved range in the current document linked to that test point

### Requirement: Test points are durable non-Gherkin verification intents

The system SHALL persist test points as project artifacts with a stable identifier, title, objective, optional natural-language preconditions and expected outcomes, hierarchy path, review state, requirement links, and scenario references. A test point SHALL NOT contain Given/When/Then steps or otherwise act as an executable Gherkin scenario.

#### Scenario: User creates a test point from selected requirement text

- **WHEN** the user creates a test point from a non-empty requirement selection
- **THEN** the system SHALL persist a `Proposed` test point linked to that exact source range
- **AND** the test point SHALL be visible in both authoring tabs

#### Scenario: Test point is edited

- **WHEN** the user edits the title, objective, preconditions, expected outcomes, or requirement links of an approved test point
- **THEN** the system SHALL change its state to `Proposed`

#### Scenario: Test point hierarchy changes

- **WHEN** the user moves a test point to a different hierarchy path without changing its verification meaning or links
- **THEN** the system SHALL preserve its current review state

### Requirement: Requirement links support resilient arbitrary text ranges

Each test-point-to-requirement link SHALL support any non-empty text range and SHALL store both character positions and a quote selector containing the exact text and surrounding context. The system SHALL use these selectors to resolve links after document edits and SHALL mark an ambiguous or missing link as stale rather than selecting unrelated text.

#### Scenario: Unchanged document resolves by position

- **WHEN** the stored document revision and text at the stored character positions match the anchor
- **THEN** the system SHALL resolve the link to that range

#### Scenario: Text moves without changing

- **WHEN** the stored position no longer matches but the exact quote and context identify one range
- **THEN** the system SHALL re-anchor the link to the uniquely matching range

#### Scenario: Quote becomes ambiguous

- **WHEN** multiple ranges match and the stored context cannot identify one uniquely
- **THEN** the system SHALL mark the link stale
- **AND** the system SHALL NOT silently choose a matching range

#### Scenario: Multibyte text is selected

- **WHEN** a link covers text containing multibyte Unicode characters
- **THEN** persisted character offsets SHALL preserve the same user-visible range after reload

### Requirement: Requirement changes invalidate affected approvals

The system SHALL re-resolve links when requirement content changes. An approved test point with any stale link SHALL become `NeedsReview`; approvals unrelated to the changed or stale ranges SHALL remain valid.

#### Scenario: Linked text changes

- **WHEN** an edit prevents an approved test point's anchor from resolving uniquely
- **THEN** the test point SHALL become `NeedsReview`

#### Scenario: Unrelated text changes

- **WHEN** a requirement edit leaves an approved test point's exact linked quotes uniquely resolvable
- **THEN** the test point SHALL remain `Approved`
- **AND** its anchors SHALL be updated to their current positions

### Requirement: The Test Points tab supports review and reverse traceability

The TUI SHALL provide a Test Points tab with the test-point hierarchy in the left pane, editable intent and review controls in the center pane, and linked requirement excerpts in the right pane.

#### Scenario: User selects a test point

- **WHEN** the user selects a test point in the tree
- **THEN** the center pane SHALL display its intent fields and review state
- **AND** the right pane SHALL display every linked requirement excerpt and its resolution state

#### Scenario: User follows a requirement excerpt

- **WHEN** the user activates a resolved requirement excerpt
- **THEN** the TUI SHALL open the Requirements tab at the linked document
- **AND** the linked range SHALL be visible and highlighted

#### Scenario: Tree groups test points

- **WHEN** test points define business-domain, function, or category hierarchy paths
- **THEN** the left pane SHALL group them by those paths rather than duplicating the requirement-document hierarchy

### Requirement: Human approval is mandatory before scenario planning

The system SHALL require an explicit human action to approve proposed or changed test points before they can be used for scenario planning. Agent approval modes, tool calls, or merely viewing a test point SHALL NOT satisfy this gate.

#### Scenario: Proposed test points await review

- **WHEN** AI generation persists proposed test points
- **THEN** the generation pipeline SHALL stop in test-point review
- **AND** it SHALL NOT invoke scenario planning until a user explicitly approves eligible test points

#### Scenario: Agent uses Auto approval mode

- **WHEN** the agent's file-change approval mode is `Auto` or `Bypass`
- **THEN** proposed test points SHALL still require explicit human approval

#### Scenario: User approves a batch

- **WHEN** the user explicitly confirms a batch of valid proposed test points
- **THEN** the system SHALL mark those test points `Approved`
- **AND** scenario planning MAY use those approved test points

#### Scenario: Included test point is not approved

- **WHEN** a scenario-planning request includes a `Proposed`, `Rejected`, `NeedsReview`, invalid, or stale test point
- **THEN** the system SHALL reject the planning request with an actionable diagnostic

### Requirement: Scenario realizations retain test-point traceability

The system SHALL allow an approved test point to be realized by one or more Gherkin scenarios and SHALL retain stable references between scenario plans, written scenarios, and their originating test points.

#### Scenario: One test point requires multiple scenarios

- **WHEN** scenario planning expands an approved test point into multiple executable variations
- **THEN** every resulting scenario SHALL reference the originating test-point identifier

#### Scenario: User follows a scenario reference

- **WHEN** a persisted test point contains a valid scenario reference
- **THEN** the TUI SHALL allow navigation to the referenced feature and scenario

#### Scenario: Existing feature has no test-point reference

- **WHEN** the TUI loads an existing Gherkin scenario without Teshi test-point metadata
- **THEN** the scenario SHALL remain usable
- **AND** the system SHALL treat it as having no authoring trace link

### Requirement: Authoring persistence fails safely

The system SHALL validate unique identities, paths, anchors, hierarchy paths, review states, and scenario references before using authoring artifacts. Writes SHALL be atomic, and malformed records SHALL produce visible diagnostics instead of being silently discarded.

#### Scenario: Duplicate test-point identifiers are loaded

- **WHEN** persisted authoring data contains duplicate test-point identifiers
- **THEN** the system SHALL report the conflict
- **AND** generation using the conflicting records SHALL be blocked

#### Scenario: Save is interrupted

- **WHEN** writing an updated test-point artifact fails before atomic replacement completes
- **THEN** the previous complete artifact SHALL remain readable
