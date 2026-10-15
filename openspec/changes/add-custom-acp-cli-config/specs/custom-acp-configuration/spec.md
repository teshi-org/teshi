## ADDED Requirements

### Requirement: Shared user-level ACP configuration
Teshi SHALL load custom and Registry-installed ACP agent definitions from one user-level JSON configuration file, SHALL preserve existing definitions that contain only `id`, `name`, `program`, `args`, and `env`, and SHALL validate every definition before use or mutation.

#### Scenario: Existing manifest remains compatible
- **WHEN** the configuration contains a previously persisted ACP agent without new optional fields
- **THEN** Teshi loads the agent with its existing program, arguments, name, and environment unchanged

#### Scenario: Invalid manual edit fails closed
- **WHEN** the configuration contains malformed JSON, duplicate IDs, an invalid ID, or an empty program
- **THEN** Teshi reports a path-qualified validation error and does not rewrite or partially accept the configuration

### Requirement: ACP configuration path discovery
The CLI SHALL expose the resolved ACP configuration path without requiring the file to exist.

#### Scenario: Print configuration path
- **WHEN** the user runs `teshi acp config-path`
- **THEN** Teshi prints the absolute path of the user-level ACP JSON configuration file

### Requirement: ACP definition inspection
The CLI SHALL list configured ACP agents and show one definition in stable human-readable or machine-readable JSON form without resolving environment references.

#### Scenario: List definitions as JSON
- **WHEN** the user runs `teshi acp list --json`
- **THEN** Teshi prints a JSON array sorted deterministically by display name and ID

#### Scenario: Show unknown definition
- **WHEN** the user requests an ID that is not configured
- **THEN** the command exits unsuccessfully with an error identifying the missing ID

### Requirement: Structured custom ACP upsert
The CLI SHALL add or replace a custom ACP definition using a validated ID, a separately supplied executable, zero or more structured arguments, an optional display name, and zero or more environment references.

#### Scenario: Add a custom agent
- **WHEN** the user runs `teshi acp add local --program agent --arg acp --name "Local Agent"`
- **THEN** Teshi resolves the executable, atomically persists the definition, and makes it available to subsequent CLI and TUI loads

#### Scenario: Update an existing ID
- **WHEN** the user runs `teshi acp add` with an ID already present
- **THEN** Teshi atomically replaces that definition without creating a duplicate

#### Scenario: Invalid executable leaves configuration unchanged
- **WHEN** the supplied executable cannot be resolved
- **THEN** the command fails and the prior configuration remains byte-for-byte unchanged

### Requirement: Runtime environment references
Custom ACP definitions SHALL support mapping child environment-variable names to host environment-variable names, SHALL resolve those references only when constructing the ACP child command, and SHALL never include resolved values in list or show output.

#### Scenario: Resolve environment reference
- **WHEN** a definition maps `API_KEY` to host variable `MY_AGENT_KEY` and `MY_AGENT_KEY` is present at launch
- **THEN** the ACP child receives `API_KEY` with the current value of `MY_AGENT_KEY`

#### Scenario: Missing referenced variable fails closed
- **WHEN** a referenced host environment variable is absent at launch
- **THEN** Teshi refuses to start that ACP agent and reports the variable names without reporting any secret value

### Requirement: ACP definition removal
The CLI SHALL remove one ACP definition atomically and SHALL fail without rewriting the file when the requested ID does not exist.

#### Scenario: Remove configured agent
- **WHEN** the user runs `teshi acp remove local` for an existing ID
- **THEN** Teshi removes only that definition and reports success

#### Scenario: Remove unknown agent
- **WHEN** the user runs `teshi acp remove missing`
- **THEN** Teshi exits unsuccessfully and leaves the configuration unchanged
