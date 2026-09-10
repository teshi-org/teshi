## ADDED Requirements

### Requirement: Validation acceptance is executable through Teshi
The repository SHALL provide paired English and `zh-CN` validation Feature files that can be executed through `teshi run` with a dedicated NDJSON runner.

#### Scenario: Both locale suites execute
- **WHEN** the CLI integration test runs the validation Feature directory for each supported locale
- **THEN** the runner SHALL emit a complete `end_run` event for each locale
- **AND** every selected scenario SHALL pass without an unimplemented step

### Requirement: The E2E runner tests a separate Teshi process
The validation runner SHALL invoke the exact Teshi binary under test as a child process and SHALL NOT call the production validation API to decide whether a scenario passed.

#### Scenario: A malformed fixture is checked by the target binary
- **WHEN** a scenario creates a malformed Feature fixture and invokes `teshi check --feature ... --json`
- **THEN** the result SHALL reflect the child-process exit status and JSON report from the target binary

#### Scenario: The runner cannot find the target binary
- **WHEN** `TESHI_BIN` is missing or points to a nonexistent path
- **THEN** the runner SHALL fail with a clear setup error instead of silently using PATH discovery

### Requirement: Source diagnostics are covered as black-box behavior
The executable validation scenarios SHALL cover missing separators, valid multilingual source, legal prose/attachments, ordered JSON diagnostics, and warning-only success.

#### Scenario: Missing Chinese step separator is reported
- **WHEN** the target binary checks a `zh-CN` fixture containing `当用户登录`
- **THEN** the scenario SHALL assert `missing_step_separator`, line 4, column 6, and suggestion `当 用户登录`

#### Scenario: Valid source and warning-only source keep their exit semantics
- **WHEN** the target binary checks valid source or source containing only authoring warnings
- **THEN** valid source SHALL have zero errors
- **AND** warning-only source SHALL exit successfully while exposing its warning code

### Requirement: Scope and execution gates are covered
The executable validation scenarios SHALL verify explicit/all check scopes, conflicting options, binding discovery failure, active-step preservation, BDD runner suppression, and replay preflight failure.

#### Scenario: Explicit and all scopes are observable
- **WHEN** a project contains valid and invalid fixtures and the target checks one fixture or all fixtures as JSON
- **THEN** the JSON scope SHALL match the requested selection
- **AND** the all-scope report SHALL contain ordered diagnostics for the invalid fixture

#### Scenario: Invalid Features fail closed before state or execution changes
- **WHEN** the target runs `steps unbound`, `steps next-unbound`, BDD run, or browser replay against an invalid fixture
- **THEN** the command SHALL fail with validation evidence
- **AND** it SHALL not return a successful empty binding result, mutate an existing active step, start the nested runner, or execute a replay action

### Requirement: E2E setup is isolated and repeatable
Each validation scenario SHALL use an isolated temporary project and SHALL clean up through temporary-directory ownership after the case completes.

#### Scenario: Repeated locale runs do not share state
- **WHEN** the English and Chinese suites run sequentially or independently
- **THEN** each suite SHALL create and inspect only its own temporary fixture and `.teshi` state
- **AND** the result SHALL not depend on a repository-local binding or installed Teshi binary
