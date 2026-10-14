## ADDED Requirements

### Requirement: Native terminal automation
Teshi SHALL provide a Rust runner that launches an executable with an argument vector, working directory, environment and terminal dimensions in a real PTY. It SHALL support text input, named keys, resizing and bounded current-screen assertions.

#### Scenario: Drive a terminal application
- **WHEN** a scenario launches an application, sends a key and waits for expected screen text
- **THEN** the runner drives the real application and passes only when the rendered current screen satisfies the assertion

#### Scenario: Invalid action
- **WHEN** a binding specifies an unsupported action or invalid dimensions
- **THEN** execution fails with an actionable error

### Requirement: Scenario integration
Teshi SHALL expose TUI execution through its CLI and scenario runner workflow with scenario selection, typed sidecar actions, standard run events and failure attachments. Missing bindings MUST fail explicitly.

#### Scenario: Execute selected scenario
- **WHEN** a user runs a selected TUI scenario
- **THEN** only that scenario and its applicable Background steps execute and results appear in the standard reporting flow

#### Scenario: Unbound step
- **WHEN** an executed step has no confirmed terminal action
- **THEN** the case fails and identifies the unbound source line

### Requirement: Bounded lifecycle and evidence
Every scenario SHALL own its terminal session, close it on success or failure, enforce finite assertion waits, and attach terminal screen evidence to assertion failures. Exit assertions SHALL observe the application exit status.

#### Scenario: Failed assertion
- **WHEN** expected text does not appear before the deadline
- **THEN** the runner reports failure, retains current screen evidence, and closes the owned session

#### Scenario: Application exit
- **WHEN** a scenario requests an exit-code assertion
- **THEN** it passes only when the owned application exits within the deadline with that code

### Requirement: Teshi TUI self-regressions
The repository SHALL include executable interface regressions that launch the actual Teshi TUI in isolated project fixtures and exercise navigation, editing and saving, and clean exit. CI and development documentation SHALL include a reproducible invocation.

#### Scenario: Edit and save a feature
- **WHEN** a self-test opens a fixture feature, edits it and saves through terminal input
- **THEN** assertions verify visible UI state and the resulting feature file contents

#### Scenario: Navigate tabs
- **WHEN** a self-test sends tab navigation keys
- **THEN** assertions verify the corresponding rendered views
