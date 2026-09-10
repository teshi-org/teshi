## ADDED Requirements

### Requirement: Core owns source-aware Gherkin diagnostics
`teshi-core` SHALL provide a deterministic, side-effect-free validation operation over raw Feature source and its permissively parsed representation, and SHALL return diagnostics with path, one-based line and column, severity, stable code, message, and optional repair suggestion.

#### Scenario: Malformed source is validated
- **WHEN** a caller validates Feature source containing syntax or structure errors
- **THEN** core SHALL return every detected diagnostic in deterministic source order without requiring filesystem, UI, agent, or runner access

#### Scenario: Valid multilingual source is validated
- **WHEN** a caller validates a structurally valid Feature in any supported Gherkin dialect
- **THEN** core SHALL use that dialect's keyword definitions and SHALL return no syntax error

#### Scenario: Valid scenario uses repeated Given steps
- **WHEN** a scenario contains multiple legal `Given`/`And` steps without a `When` or `Then`
- **THEN** validation SHALL NOT report an error solely because the scenario lacks those keyword types
- **AND** any completeness guidance SHALL be a warning or suggestion that does not block binding or execution

### Requirement: Missing step separators are diagnosed
The validator SHALL report an error with code `missing_step_separator` when a non-comment source line begins with an active-dialect step keyword that is immediately followed by non-whitespace instead of a required separator.

#### Scenario: Chinese step omits its separator
- **WHEN** a `zh-CN` Scenario contains `当用户登录`
- **THEN** validation SHALL report `missing_step_separator` at that line and at the end of the `当` keyword
- **AND** the suggestion SHALL show an equivalent step beginning `当 用户登录`

#### Scenario: English step has a separator
- **WHEN** an English Scenario contains `When the user logs in`
- **THEN** validation SHALL recognize the line as a step and SHALL NOT report `missing_step_separator`

#### Scenario: Longer dialect keyword shares a prefix
- **WHEN** multiple active-dialect keywords can prefix the same source line
- **THEN** validation SHALL evaluate the longest matching keyword before producing a diagnostic or suggestion

### Requirement: Valid prose and attachments remain supported
The validator SHALL distinguish executable structure from legal Feature and Scenario descriptions, comments, tags, data tables, and DocStrings so permissive authoring content is not rejected merely because it is not a step.

#### Scenario: Scenario begins with description prose
- **WHEN** a Scenario contains valid descriptive prose before its first recognized step
- **THEN** validation SHALL preserve the prose and SHALL NOT report it as a syntax error

#### Scenario: Unrecognized line appears after steps begin
- **WHEN** a Scenario has begun its executable steps and then contains non-empty text that is neither a recognized step nor a valid attachment or comment
- **THEN** validation SHALL report an error identifying the unrecognized executable-region line

#### Scenario: Step includes a table or DocString
- **WHEN** a recognized step is followed by a syntactically valid data table or DocString
- **THEN** validation SHALL accept the attachment and SHALL NOT report its content as unrecognized lines

### Requirement: Parser remains permissive
The existing Feature parser SHALL continue returning a renderable partial AST for incomplete or malformed source, while validation SHALL report correctness independently.

#### Scenario: Editor contains an incomplete step
- **WHEN** an editor parses a buffer containing a malformed step-looking line
- **THEN** the parser SHALL still return content suitable for editor rendering
- **AND** the validator SHALL return the corresponding error diagnostic

### Requirement: Check command validates explicit scopes
Teshi SHALL provide `teshi check`, `teshi check --feature <path>`, and `teshi check --all`; `--feature` and `--all` SHALL be mutually exclusive, and the command SHALL support a machine-readable JSON report.

#### Scenario: One Feature is checked
- **WHEN** a user runs `teshi check --feature features/zh-CN/login.feature`
- **THEN** Teshi SHALL validate only that Feature and print its diagnostics with source locations

#### Scenario: All Features are checked
- **WHEN** a user runs `teshi check --all`
- **THEN** Teshi SHALL discover and validate all Feature files in project scope and print an aggregate summary

#### Scenario: JSON output is requested
- **WHEN** a caller runs `teshi check --all --json`
- **THEN** Teshi SHALL return a stable JSON envelope containing scope, summary counts, and ordered diagnostics

#### Scenario: Validation contains errors
- **WHEN** a completed check report contains one or more error diagnostics
- **THEN** the command SHALL exit unsuccessfully while still emitting the complete report

#### Scenario: Validation contains warnings only
- **WHEN** a completed check report contains warnings or suggestions but no errors
- **THEN** the command SHALL exit successfully

#### Scenario: Directory execution validates only its selected directory
- **WHEN** a user runs BDD execution against a Feature directory inside a larger project
- **THEN** validation SHALL cover the same directory scope from which runnable scenarios are collected
- **AND** an invalid Feature outside that selected directory SHALL NOT block the run

### Requirement: Binding discovery fails closed on invalid Features
Step commands that discover, select, or resolve executable Feature steps SHALL validate their selected Feature scope before reporting binding state or changing selection state.

#### Scenario: Unbound command receives a malformed Feature
- **WHEN** `teshi steps unbound` targets a Feature with an error diagnostic
- **THEN** the command SHALL fail with the validation report and SHALL NOT return an empty list as a successful binding result

#### Scenario: Next-unbound command receives a malformed Feature
- **WHEN** `teshi steps next-unbound` targets a Feature with an error diagnostic
- **THEN** the command SHALL fail before writing or changing the active-step state

#### Scenario: Binding command receives warnings only
- **WHEN** a selected Feature has warnings but no error diagnostics
- **THEN** the command SHALL continue and SHALL make the warnings available to the caller
- **AND** direct CLI, REST, and control-channel callers SHALL receive semantically equivalent warning reports

### Requirement: BDD execution fails closed on invalid Features
Teshi BDD run and browser or WinApp replay entry points SHALL validate every Feature in their selected execution scope before starting a runner or mutating target application state.

#### Scenario: Run scope contains an invalid Feature
- **WHEN** a user starts BDD execution for a scope containing an error diagnostic
- **THEN** Teshi SHALL emit the validation report and SHALL NOT start the runner

#### Scenario: Replay Feature is invalid
- **WHEN** browser or WinApp replay targets a Feature containing an error diagnostic
- **THEN** Teshi SHALL fail before executing any bound action

### Requirement: Direct and daemon-backed validation have parity
Direct-file CLI paths and daemon-backed command paths SHALL use the same core validator, severity gate, diagnostic codes, and source locations.

#### Scenario: Live daemon changes steps routing
- **WHEN** the same invalid Feature is checked through direct and live-daemon steps paths
- **THEN** both paths SHALL reject it with semantically equivalent diagnostics and SHALL perform no binding mutation

#### Scenario: Daemon validation errors remain structured
- **WHEN** a daemon-gated operation rejects a Feature with validation errors
- **THEN** REST and control-channel responses SHALL preserve a machine-readable validation report rather than embedding it only as a JSON string

### Requirement: Editors present diagnostics without blocking authoring
Teshi editor surfaces SHALL be able to request diagnostics for the current buffer while continuing to display, navigate, edit, and save partial Feature content.

#### Scenario: User is midway through a malformed step
- **WHEN** an editor buffer has an error diagnostic during active authoring
- **THEN** the editor SHALL keep the buffer available for editing and SHALL present the diagnostic without replacing the permissive parse result

### Requirement: Agent and automation delegate to Teshi validation
Agent tools, packaged Skills, and CI integrations SHALL consume the core diagnostics through Teshi adapters and SHALL NOT maintain independent dialect syntax rules.

#### Scenario: Agent validates a generated Feature
- **WHEN** an agent or Skill checks generated Feature content before binding
- **THEN** it SHALL invoke the Teshi validation contract and use its diagnostic codes and severity result

#### Scenario: Validation behavior changes
- **WHEN** a dialect or syntax rule changes in `teshi-core`
- **THEN** CLI, agent, Skill, editor, and CI consumers SHALL receive the new behavior without a duplicate rule update in those consumers
