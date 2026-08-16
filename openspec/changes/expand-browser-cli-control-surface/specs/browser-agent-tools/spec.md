## MODIFIED Requirements

### Requirement: browser_snapshot tool
The system SHALL provide a `browser_snapshot` tool that uses the shared typed snapshot operation and returns parsed interactive elements plus revision-bound references for the explicitly targeted leased browser session. Legacy implicit selection SHALL be allowed only when exactly one eligible target exists.

#### Scenario: Agent inspects the current page
- **WHEN** the agent calls `browser_snapshot` with a complete target and valid lease
- **THEN** the tool SHALL return the structured snapshot, page revision, and target-scoped element references

#### Scenario: Several profiles are eligible
- **WHEN** the agent calls `browser_snapshot` without a target and several sessions are eligible
- **THEN** the tool SHALL return `ambiguous_browser_target` and SHALL NOT inspect any page

### Requirement: browser_click tool
The system SHALL provide a `browser_click` tool that uses the shared typed action operation, accepts a current element reference or structured locator candidate, requires explicit target and lease in multi-profile use, and returns separate action and optional wait results.

#### Scenario: Agent clicks a referenced element
- **WHEN** the agent calls `browser_click` with a current reference, complete target, and valid lease
- **THEN** the tool SHALL execute the declared DOM or pointer activation once and return the correlated result

### Requirement: browser_type tool
The system SHALL provide a `browser_type` tool that uses the shared typed action operation and requires a current reference or structured candidate, explicit text, complete target, and valid lease.

#### Scenario: Agent types into a referenced element
- **WHEN** the agent calls `browser_type` with valid target, ownership, reference, and text
- **THEN** the tool SHALL focus the resolved element, perform the advertised backend action, and return success or a stable action error

### Requirement: browser_assert tool
The system SHALL provide a `browser_assert` tool that evaluates typed text, URL, element-state, and revision conditions against an explicitly targeted leased page without performing a mutation.

#### Scenario: text_visible assertion
- **WHEN** the agent calls `browser_assert("text_visible", "Welcome")` against a selected target
- **THEN** the tool SHALL report whether `Welcome` is visible in that page context

#### Scenario: url_match assertion
- **WHEN** the agent calls `browser_assert("url_match", "/dashboard")` against a selected target
- **THEN** the tool SHALL report whether the selected tab URL contains `/dashboard`

### Requirement: browser_go_back tool
The system SHALL provide a `browser_go_back` tool that uses the shared typed navigation operation and requires the selected profile lease before changing history.

#### Scenario: Agent navigates to the previous page
- **WHEN** the agent calls `browser_go_back` with a complete target and valid lease
- **THEN** the tool SHALL navigate only that tab and return its new page-context revision

## ADDED Requirements

### Requirement: Shared browser tool registration
All browser tools SHALL be registered from the shared operation schemas with explicit capability, target, lease, timeout, and sensitive-output annotations.

#### Scenario: LLM tool schema is assembled
- **WHEN** the agent's LLM tool schema is built for a selected policy
- **THEN** it SHALL include only operations enabled by that policy and SHALL retain the shared input validation rules
