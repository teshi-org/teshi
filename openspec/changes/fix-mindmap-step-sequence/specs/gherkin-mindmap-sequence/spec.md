## ADDED Requirements

### Requirement: Preserve scenario step order
The Gherkin MindMap SHALL represent each Scenario as an ordered path whose nodes follow the Background and Scenario steps in source order, regardless of Given, When, Then, And, or But keyword transitions.

#### Scenario: Repeated action and result groups remain one path
- **WHEN** a Scenario contains `When action one`, `Then result one`, `When action two`, and `Then result two` in that order
- **THEN** the MindMap SHALL place each step beneath the immediately preceding step as one continuous path

#### Scenario: Consecutive result steps remain ordered
- **WHEN** a Scenario contains a Then step followed by Then-effective And and But steps
- **THEN** the MindMap SHALL preserve those steps as consecutive parent-child nodes in source order

#### Scenario: Background precedes scenario steps
- **WHEN** a Feature has ordered Background steps and a Scenario has ordered Scenario steps
- **THEN** the MindMap SHALL prefix the Scenario path with the Background steps in source order

### Requirement: Share only ordered prefixes
The Gherkin MindMap SHALL merge equal step text reached through the same ordered parent path and SHALL branch when Scenario paths first differ.

#### Scenario: Scenarios share a long prefix
- **WHEN** two Scenarios contain the same ordered Background and initial Scenario steps
- **THEN** the MindMap SHALL represent the shared prefix once and record source locations from both Scenarios on its nodes

#### Scenario: Scenarios diverge after a shared prefix
- **WHEN** two Scenarios differ after an equal ordered prefix
- **THEN** the first differing steps SHALL be sibling branches beneath the final shared node

#### Scenario: Rule-nested scenario contributes an ordered path
- **WHEN** a Rule contains a Scenario with an ordered step sequence
- **THEN** that Scenario SHALL contribute a source-ordered path to the same MindMap index
