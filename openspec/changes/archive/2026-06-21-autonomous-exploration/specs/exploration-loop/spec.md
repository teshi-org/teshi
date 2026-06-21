## ADDED Requirements

### Requirement: Gherkin-scenario-driven exploration
The exploration loop SHALL accept a Gherkin scenario context (feature path, scenario line range, step line numbers and texts) as its input. The agent SHALL use the scenario's Given/When/Then steps as the mission to fulfill in the browser.

#### Scenario: Agent explores from a login scenario
- **WHEN** the agent is given a scenario with steps "Given I am on the login page", "When I enter valid credentials", "Then I should see the dashboard"
- **THEN** the agent SHALL navigate to the login page, find the username/password fields, and verify the dashboard appears

#### Scenario: Agent correlates step text to page elements
- **WHEN** the agent reads step text "I click the 'Save' button"
- **THEN** the agent SHALL look for a button with text "Save" in the page snapshot and interact with it

### Requirement: ReAct exploration cycle
The system SHALL run an agent loop that repeatedly executes observe → think → act cycles, where the agent observes the page snapshot, decides the next action relative to the current Gherkin step, and executes it via browser tools.

#### Scenario: Agent completes a multi-step Gherkin scenario
- **WHEN** the agent is given a 3-step Gherkin scenario
- **THEN** the agent SHALL complete all steps and produce a trace with one binding per step

#### Scenario: Agent recovers from failed action
- **WHEN** a browser tool call returns an error (element not found, navigation failed)
- **THEN** the agent SHALL receive the error and attempt an alternative action before moving to the next step

### Requirement: Maximum step limit
The exploration loop SHALL enforce a configurable maximum number of steps per scenario, defaulting to 15. When exceeded, the loop SHALL terminate and report failure.

#### Scenario: Loop terminates at step limit
- **WHEN** the agent reaches the maximum step count without completing the scenario
- **THEN** the exploration SHALL terminate and the trace SHALL be marked as incomplete

### Requirement: URL sandbox
The exploration loop SHALL enforce a URL whitelist. If the browser navigates to a URL outside the whitelist, the loop SHALL execute `goBack` and record a boundary violation.

#### Scenario: Agent navigates outside whitelist
- **WHEN** the browser navigates to a URL that does not match the whitelist patterns
- **THEN** the browser SHALL navigate back and the agent SHALL be notified of the violation

### Requirement: Dirty data isolation
The exploration loop SHALL provide a `reset_environment` tool that restores the application to a known clean state before exploration begins.

#### Scenario: Environment is reset before exploration
- **WHEN** the agent starts a new exploration session
- **THEN** `reset_environment` SHALL be called to ensure a clean starting state

### Requirement: Full trace recording
The exploration loop SHALL record every action with its timestamp, page URL, element ref, action type, action arguments, the resulting page snapshot, and the Gherkin step line it was fulfilling.

#### Scenario: Trace captures all actions with step context
- **WHEN** the agent completes or terminates an exploration
- **THEN** the trace SHALL contain an ordered list of every action with full metadata, each tagged with the target `step_line` from the feature file
