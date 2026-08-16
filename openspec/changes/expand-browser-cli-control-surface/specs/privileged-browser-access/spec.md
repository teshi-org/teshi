## ADDED Requirements

### Requirement: Privileged capabilities default to denied
Arbitrary JavaScript, raw CDP, Cookie access, content-setting changes, and extension-management operations SHALL be disabled unless the exact capability has an active explicit grant.

#### Scenario: Caller invokes JavaScript without a grant
- **WHEN** a caller requests arbitrary JavaScript with only a valid browser lease
- **THEN** Teshi SHALL return `browser_capability_denied` and SHALL execute no script

### Requirement: Scoped short-lived capability grants
Teshi SHALL bind privileged grants to the local user, broker instance, project, extension instance, capability name, caller label, and bounded expiry.

#### Scenario: Grant is reused after broker restart
- **WHEN** a caller presents a grant issued by a previous broker instance
- **THEN** Teshi SHALL reject it and require a new grant

### Requirement: Explicit non-interactive policy
Non-interactive privileged grants SHALL require both a project or user policy allowlist and an explicit command-line acknowledgement naming the requested capability.

#### Scenario: CI requests raw CDP without policy
- **WHEN** a non-interactive caller requests a raw-CDP grant but policy does not allow it
- **THEN** Teshi SHALL fail closed without prompting indefinitely or broadening the grant

### Requirement: Optional browser permission gate
Privileged operations that require Chromium optional permissions SHALL remain unavailable until the user approves the exact permission through an extension user gesture.

#### Scenario: Cookie permission is not approved
- **WHEN** a valid Teshi grant requests Cookie access but the extension lacks Cookie permission
- **THEN** Teshi SHALL return `browser_capability_unavailable` with local approval guidance and SHALL NOT request broader permissions silently

### Requirement: Audited privileged execution
Each privileged request SHALL append a metadata-only audit record containing timestamp, capability, caller label, target, request ID, outcome, and redacted argument summary.

#### Scenario: Raw CDP command completes
- **WHEN** an authorized raw CDP operation succeeds or fails
- **THEN** Teshi SHALL record the domain and method but SHALL omit sensitive parameter and response bodies from the default audit log

### Requirement: Bounded arbitrary JavaScript
Authorized JavaScript execution SHALL be target-scoped, timeout-bounded, result-size-bounded, and correlated to the selected page revision when one is supplied.

#### Scenario: Script exceeds its deadline
- **WHEN** an authorized script does not complete before its timeout
- **THEN** Teshi SHALL cancel or abandon the pending request, return `browser_operation_timeout`, and prevent its response from satisfying another request

### Requirement: Allowlisted raw CDP domains
Raw CDP execution SHALL enforce an effective domain/method allowlist derived from policy and grant scope and SHALL deny browser-level, target-escaping, script-execution, Cookie, upload, download, or stream methods governed by separate capability contracts.

#### Scenario: Page-scoped grant requests a browser-level method
- **WHEN** a raw-CDP caller requests a method outside its effective allowlist
- **THEN** Teshi SHALL return `browser_capability_denied` before sending the command to Chromium

#### Scenario: Raw CDP policy includes a separately gated method
- **WHEN** policy names a Runtime, Cookie, file-input, or download method under only a raw-CDP grant
- **THEN** Teshi SHALL return `browser_capability_denied` without bypassing the JavaScript, Cookie, upload, or artifact capability boundary

### Requirement: Cookie minimization
Cookie access SHALL default to names and metadata with values redacted and SHALL require a distinct value-access scope to return Cookie values.

#### Scenario: Caller has metadata-only Cookie grant
- **WHEN** the caller lists Cookies for the selected tab
- **THEN** Teshi SHALL return scoped Cookie metadata with values redacted

### Requirement: Privileged MCP exposure is opt-in
The local MCP server SHALL omit P2 tools unless startup configuration explicitly allowlists them and the active policy permits no broader scope.

#### Scenario: MCP starts with default settings
- **WHEN** the MCP server advertises its tools without a privileged allowlist
- **THEN** arbitrary JavaScript, raw CDP, Cookie, content-setting, and extension-management tools SHALL be absent
