## ADDED Requirements

### Requirement: browser_snapshot tool
The system SHALL provide an LLM-callable tool that returns a structured snapshot of the current browser page, including all interactive elements with their `teshi-id`, role, accessible name, and element type.

#### Scenario: Agent gets page snapshot
- **WHEN** the agent calls `browser_snapshot`
- **THEN** the response SHALL contain a list of interactive elements with their ref IDs, roles, names, and types

#### Scenario: Snapshot includes URL and title
- **WHEN** the agent calls `browser_snapshot`
- **THEN** the response SHALL include the current page URL and document title

### Requirement: browser_click tool
The system SHALL provide an LLM-callable tool that clicks an element identified by `teshi-id`.

#### Scenario: Agent clicks element by ref
- **WHEN** the agent calls `browser_click(ref="e15")`
- **THEN** the element with `teshi-id="e15"` SHALL receive a click event

#### Scenario: Click on non-existent ref returns error
- **WHEN** the agent calls `browser_click(ref="e999")`
- **THEN** the tool SHALL return an error indicating the ref was not found

### Requirement: browser_type tool
The system SHALL provide an LLM-callable tool that types text into an input element identified by `teshi-id`.

#### Scenario: Agent types into input
- **WHEN** the agent calls `browser_type(ref="e22", text="admin")`
- **THEN** the element with `teshi-id="e22"` SHALL receive the text "admin" as input value

### Requirement: browser_assert tool
The system SHALL provide an LLM-callable tool that checks a condition on the current page (text visible, element present, URL matches).

#### Scenario: Agent asserts text is visible
- **WHEN** the agent calls `browser_assert(condition="text=Dashboard", type="visible")`
- **THEN** the tool SHALL return success if the text "Dashboard" is visible on the page

#### Scenario: Agent asserts URL matches
- **WHEN** the agent calls `browser_assert(condition=".*/dashboard", type="url_match")`
- **THEN** the tool SHALL return success if the current URL matches the regex

### Requirement: browser_go_back tool
The system SHALL provide an LLM-callable tool that navigates the browser back one page in history.

#### Scenario: Agent navigates back
- **WHEN** the agent calls `browser_go_back`
- **THEN** the browser SHALL execute `window.history.back()`
