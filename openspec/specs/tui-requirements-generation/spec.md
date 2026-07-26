# TUI Requirements Generation

## Purpose

Requirements gathering and test-point generation happen in the TUI. Test points are Gherkin scenarios and steps written to `.feature` files via the Agent generation pipeline, browsable in the TUI Gherkin MindMap. FreeMind `.mm` and mock HTML are not generation products.

## Requirements

### Requirement: TUI owns requirements gathering for feature generation

The TUI SHALL support gathering free-text or conversational requirements via the AI Agent generation pipeline. When the user asks to create or generate a feature, the agent SHALL follow the staged pipeline (requirements gathering, planning, writing) and SHALL call `submit_requirements` before planning and writing.

#### Scenario: User starts generation from chat

- **WHEN** the user asks the TUI agent to create a feature from requirements
- **THEN** the agent SHALL enter requirements gathering and SHALL NOT skip directly to writing files without submitting requirements

#### Scenario: Requirements can include pasted text

- **WHEN** the user pastes multi-line requirement text into the TUI AI input
- **THEN** the system SHALL accept the paste and make the text available to the agent conversation

### Requirement: Test points are Gherkin scenarios and steps

The system SHALL treat generated test points as Gherkin scenarios and steps written to `.feature` files. The system SHALL NOT require FreeMind `.mm` documents or mock HTML as generation products. Users SHALL be able to browse generated scenarios in the TUI Gherkin MindMap after features are written and reloaded.

#### Scenario: Generation writes feature files

- **WHEN** the generation pipeline completes the writing stage successfully
- **THEN** the project SHALL contain one or more `.feature` files reflecting the planned scenarios

#### Scenario: No FreeMind product required

- **WHEN** the user completes requirements-to-test-point generation in the TUI
- **THEN** the system SHALL NOT require writing `requirements.mm` or `mock.html` under `.teshi/testpoints/`

### Requirement: Generation pipeline stages remain authoritative

The TUI Agent generation pipeline SHALL remain the authoritative path from requirements to executable scenarios: Gathering → Planning → Writing (with confirmation/validation as implemented). Intermediate structured plans SHALL use the existing pipeline tools (`submit_requirements`, `generate_plan`, feature mutation tools), not a FreeMind XML tool.

#### Scenario: Plan follows requirements submission

- **WHEN** `submit_requirements` has been recorded
- **THEN** the pipeline stage SHALL advance toward planning and the agent SHALL be guided to call `generate_plan` before writing features
