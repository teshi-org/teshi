## ADDED Requirements

### Requirement: Distilled locator → step binding mapping
The system SHALL map each distilled locator from the exploration trace to the corresponding Gherkin step line in the feature file, producing entries in the existing `.teshi/step-bindings/{feature}.json` format.

#### Scenario: Binding written for each explored step
- **WHEN** an exploration trace has actions tagged with `step_line` values
- **THEN** the system SHALL produce one `StepBinding` entry per step line, containing the distilled primary locator

### Requirement: StepBinding format conformance
The generated bindings SHALL conform to the existing `StepBinding` schema: `{ step_line, step_keyword, step_text, step_text_normalized, source: "agent", status: "confirmed", primary: { strategy, value, action }, confirmed_at }`.

#### Scenario: Binding matches existing schema
- **WHEN** a binding is written to `.teshi/step-bindings/{feature}.json`
- **THEN** its structure SHALL be identical to bindings created by the manual bdd-locator workflow

### Requirement: Source tracking
Generated bindings SHALL set `source` to `"agent"` to distinguish AI-discovered bindings from manually recorded ones.

#### Scenario: Source field distinguishes agent bindings
- **WHEN** a user inspects step binding statuses
- **THEN** agent-generated bindings SHALL have `source: "agent"` and manually recorded ones `source: "binding"`

### Requirement: Incremental binding update
The system SHALL read existing step bindings for the feature before writing, preserving any manually confirmed bindings and only overwriting steps that were explored by the agent.

#### Scenario: Manual bindings preserved
- **WHEN** a feature file has 5 steps, 3 already bound manually, and the agent explores all 5
- **THEN** the 3 manual bindings SHALL remain unchanged and only the 2 unbound steps SHALL receive agent-generated bindings
