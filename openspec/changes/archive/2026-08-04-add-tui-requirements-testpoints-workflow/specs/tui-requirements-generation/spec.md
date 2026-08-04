## MODIFIED Requirements

### Requirement: TUI owns requirements gathering for feature generation

The TUI SHALL support gathering free-text or conversational requirements through the AI Agent generation pipeline. When the user asks to create or generate a feature, the agent SHALL gather requirements, propose non-Gherkin test points, wait for explicit human test-point approval, plan scenarios from approved test points, and only then write `.feature` files.

#### Scenario: User starts generation from chat

- **WHEN** the user asks the TUI agent to create a feature from requirements
- **THEN** the agent SHALL enter requirements gathering
- **AND** it SHALL NOT skip directly to test-point proposal, scenario planning, or file writing without submitting requirements

#### Scenario: Requirements can include pasted text

- **WHEN** the user pastes multi-line requirement text into the TUI AI input
- **THEN** the system SHALL accept the paste and make the text available to the agent conversation
- **AND** the user SHALL be able to persist the accepted text as a requirement document

#### Scenario: Requirements are selected from project documents

- **WHEN** the user starts generation from one or more persisted requirement documents or ranges
- **THEN** the pipeline SHALL retain those document and range identities as generation sources

### Requirement: Generation pipeline stages remain authoritative

The TUI Agent generation pipeline SHALL remain the authoritative path from requirements to executable scenarios. Its ordered phases SHALL be Gathering → Generating Test Points → Reviewing Test Points → Planning → Writing, followed by confirmation and validation as applicable. Intermediate test points and plans SHALL use Teshi's structured pipeline tools and persisted authoring artifacts, not FreeMind XML or generated mock HTML.

#### Scenario: Test-point proposal follows requirements submission

- **WHEN** `submit_requirements` has been recorded
- **THEN** the pipeline SHALL advance to test-point generation
- **AND** the agent SHALL be guided to call `propose_test_points`

#### Scenario: Proposed test points await human review

- **WHEN** `propose_test_points` persists a valid proposal
- **THEN** the pipeline SHALL enter Reviewing Test Points
- **AND** the agent SHALL not call `generate_plan` until the TUI records explicit human approval

#### Scenario: Approved test points advance to planning

- **WHEN** a human explicitly approves at least one valid test point and chooses to continue generation
- **THEN** the pipeline SHALL advance to Planning
- **AND** planning SHALL be limited to approved test-point identifiers

#### Scenario: Scenario plan follows approved test points

- **WHEN** `generate_plan` receives only approved, resolved test-point identifiers
- **THEN** the pipeline SHALL record their scenario realizations and advance toward Writing

### Requirement: Generation state survives TUI restart

The TUI SHALL reconstruct the current requirement selection, proposed and reviewed test points, and the latest generation plan from persisted project artifacts and resumable agent-session state. Restarting SHALL NOT implicitly approve test points or skip a required review phase.

#### Scenario: TUI closes during test-point review

- **WHEN** the project is reopened while proposed test points remain unresolved
- **THEN** the TUI SHALL restore the Reviewing Test Points phase
- **AND** the same proposed test points SHALL remain unapproved

#### Scenario: Requirement changes before generation resumes

- **WHEN** persisted requirement content changes after test-point approval but before scenario planning
- **THEN** the system SHALL resolve affected anchors again
- **AND** it SHALL return affected test points to human review when required

## REMOVED Requirements

### Requirement: Test points are Gherkin scenarios and steps

**Reason**: Test points are now a durable, non-executable review layer between requirement text and Gherkin scenario planning. Treating scenarios as test points prevents independent review and requirement traceability.

**Migration**: Existing `.feature` scenarios remain valid but have no test-point trace links. New generation creates approved test-point artifacts before writing linked scenarios.
