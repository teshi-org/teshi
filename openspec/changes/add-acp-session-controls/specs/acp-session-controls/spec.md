## ADDED Requirements

### Requirement: Discover all advertised session settings
The client SHALL retain all configuration options from session creation/loading and configuration updates, including names, descriptions, categories, values, grouping, and types. It MUST support select and boolean options and show unknown types as unsupported without inventing choices.

#### Scenario: Modern configuration discovery
- **WHEN** an agent returns model, mode, thought level, boolean speed, and an unfamiliar category
- **THEN** all options remain accessible with the agent's labels and current values

#### Scenario: Legacy agent compatibility
- **WHEN** an agent advertises legacy modes or models without an equivalent modern category
- **THEN** the client offers those controls and changes them through the corresponding legacy method

### Requirement: Change configuration on the current session
The runtime SHALL validate and serialize configuration changes against the current session and SHALL display confirmed values only. It MUST apply the authoritative returned configuration list and process idle notifications.

#### Scenario: Model changes reasoning choices
- **WHEN** a successful model change returns revised reasoning choices
- **THEN** the model and reasoning controls reflect the returned list and stale choices cannot be submitted

#### Scenario: Rejected or uncertain change
- **WHEN** a change is rejected or times out
- **THEN** the UI reports the error without claiming the requested value is active, and a timeout requires state reconciliation before further changes

#### Scenario: Session isolation
- **WHEN** a user switches conversations while a configuration response is pending
- **THEN** that response cannot change the newly selected conversation's settings

### Requirement: Clickable and keyboard-accessible bottom controls
The TUI SHALL expose session controls next to the composer, allow discovery before the first prompt, and support mouse selection and keyboard navigation. Narrow layouts MUST retain access to every setting through overflow. Busy and pending states MUST be visible and prevent conflicting edits.

#### Scenario: Select a model
- **WHEN** the user clicks the model control and selects an available entry
- **THEN** the existing session receives the change and the bottom row displays its confirmed result

#### Scenario: Narrow terminal and keyboard access
- **WHEN** the terminal cannot fit all controls
- **THEN** an overflow picker makes every option reachable and keyboard users can select, confirm, or dismiss without sending a prompt

### Requirement: Effective approval policy is explicit
The UI SHALL distinguish agent mode from Teshi's tool approval policy and SHALL display the effective policy, including overrides. Policy changes MUST affect subsequent permission requests without changing pending decisions or mandatory review gates.

#### Scenario: Change approvals after connecting
- **WHEN** the user changes Teshi approvals on a connected idle session
- **THEN** the next permission request uses the displayed policy and an already pending request is not automatically approved

### Requirement: Evidence-based acceptance
Validation SHALL separately report deterministic protocol/runtime/UI tests and real-agent terminal acceptance.

#### Scenario: Automated tests pass without a real terminal run
- **WHEN** all fixtures pass but live interaction has not been exercised
- **THEN** real-agent and visible TUI acceptance remain incomplete
