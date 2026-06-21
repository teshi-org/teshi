# Browser Agent Tools

Defines the tool surface that the LLM agent uses to interact with the browser:
snapshot inspection, clicking, typing, assertion, and navigation.

## Purpose

Provide a set of structured tools (`browser_snapshot`, `browser_click`,
`browser_type`, `browser_assert`, `browser_go_back`) registered in the LLM
tool schema so the agent can observe and manipulate the browser during
autonomous exploration.

## ADDED Requirements

### Requirement: browser_snapshot tool
The system SHALL provide a `browser_snapshot` tool that calls `get_structured_snapshot` over the WebSocket and returns a parsed element list to the LLM.

### Requirement: browser_click tool
The system SHALL provide a `browser_click(ref)` tool that sends a `click_ref` command over the WebSocket and returns success or error.

### Requirement: browser_type tool
The system SHALL provide a `browser_type(ref, text)` tool that sends a `type_ref` command over the WebSocket and returns success or error.

### Requirement: browser_assert tool
The system SHALL provide a `browser_assert(condition_type, value)` tool that checks text visibility or URL match on the current page state.

#### Scenario: text_visible assertion
- **WHEN** the agent calls `browser_assert("text_visible", "Welcome")`
- **THEN** the tool SHALL check if "Welcome" text is visible on the page

#### Scenario: url_match assertion
- **WHEN** the agent calls `browser_assert("url_match", "/dashboard")`
- **THEN** the tool SHALL check if the current URL contains "/dashboard"

### Requirement: browser_go_back tool
The system SHALL provide a `browser_go_back` tool that sends a navigation back command over the WebSocket.

### Requirement: Tool registration
All browser tools SHALL be registered in the LLM tool schema with JSON Schema definitions so the LLM discovers and can call them.
