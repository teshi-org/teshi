# Browser Agent Tools

Defines the tool surface that the LLM agent uses to interact with the browser:
snapshot inspection, clicking, typing, assertion, and navigation.

## Purpose

Provide a set of structured tools (`browser_snapshot`, `browser_click`,
`browser_type`, `browser_assert`, `browser_go_back`) registered in the LLM
tool schema so the agent can observe and manipulate the browser during
autonomous exploration.

## Requirements

### Requirement: browser_snapshot tool

The system SHALL provide a `browser_snapshot` tool that calls `get_structured_snapshot` over the WebSocket and returns a parsed element list to the LLM.

#### Scenario: Agent inspects the current page

- **WHEN** the agent calls `browser_snapshot`
- **THEN** the tool SHALL request a structured snapshot and return the parsed interactive elements

### Requirement: browser_click tool

The system SHALL provide a `browser_click(ref)` tool that sends a `click_ref` command over the WebSocket and returns success or error.

#### Scenario: Agent clicks a referenced element

- **WHEN** the agent calls `browser_click` with a snapshot element reference
- **THEN** the tool SHALL send `click_ref` for that reference and return the operation result

### Requirement: browser_type tool

The system SHALL provide a `browser_type(ref, text)` tool that sends a `type_ref` command over the WebSocket and returns success or error.

#### Scenario: Agent types into a referenced element

- **WHEN** the agent calls `browser_type` with an element reference and text
- **THEN** the tool SHALL send `type_ref` with both values and return the operation result

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

#### Scenario: Agent navigates to the previous page

- **WHEN** the agent calls `browser_go_back`
- **THEN** the tool SHALL send the navigation-back command and return the operation result

### Requirement: Tool registration

All browser tools SHALL be registered in the LLM tool schema with JSON Schema definitions so the LLM discovers and can call them.

#### Scenario: LLM tool schema is assembled

- **WHEN** the agent's LLM tool schema is built
- **THEN** it SHALL contain each browser tool with its JSON Schema definition
