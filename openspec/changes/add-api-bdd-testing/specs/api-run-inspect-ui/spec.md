## ADDED Requirements

### Requirement: TUI Explore HTTP inspector

When a run produces `http_exchange` events, the TUI Explore view SHALL allow the user to select a Gherkin step and inspect each exchange for that step, including redacted request and response envelopes and assertion results. The user SHALL be able to expand redacted fields to plaintext in the inspector.

#### Scenario: User inspects a failed API step

- **WHEN** an `[API]` step fails an envelope assert
- **THEN** Explore SHALL show the step as failed and display the exchange including the failed assertion text

### Requirement: TUI can run API and mixed scenarios

The TUI SHALL run `@api` and mixed `@api` `@ui` scenarios using the Teshi dispatcher and API sidecar as specified by `api-bdd-testing`, and SHALL stream step and exchange events into Explore.

#### Scenario: User runs a mixed scenario from Explore

- **WHEN** the user runs a Scenario tagged `@api` `@ui`
- **THEN** Explore SHALL show both UI step results and API exchanges in scenario order

### Requirement: GPUI run and inspect surface

The shared GPUI shell SHALL provide a Run/API surface that lists scenarios from the open project, starts a run, and displays steps and `http_exchange` trees. That surface MUST NOT include a Gherkin editor.

#### Scenario: GPUI shows an exchange after run

- **WHEN** the user starts a pure `@api` scenario from the GPUI Run surface
- **THEN** the surface SHALL list each step and each HTTP exchange emitted for those steps

#### Scenario: GPUI does not edit feature files

- **WHEN** the user uses only the GPUI Run/API surface
- **THEN** the surface SHALL NOT persist edits to `.feature` files

### Requirement: Shared event schema for TUI and GPUI

TUI Explore and the GPUI Run surface SHALL consume the same `http_exchange` and step events (NDJSON and/or sidecar WebSocket as implemented by the engine/daemon).

#### Scenario: Same run visible in both shells

- **WHEN** a daemon-hosted web shell and the TUI could observe the same project run stream
- **THEN** both SHALL interpret an `http_exchange` payload with the same field names defined in `api-bdd-testing`
