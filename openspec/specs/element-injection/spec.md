# Element Injection

Automatically assigns stable `teshi-id` attributes to interactive DOM elements
via CDP script injection, enabling the LLM agent to reference elements by ID.

## Purpose

Provide a consistent, LLM-friendly way to identify and reference interactive
elements in the browser during autonomous exploration.

## ADDED Requirements

### Requirement: Interactive element identification
The system SHALL traverse the DOM at page load and assign a `teshi-id` attribute to every interactive element.

#### Scenario: Buttons receive teshi-id
- **WHEN** a page loads with a `<button>Submit</button>` element
- **THEN** the button SHALL have a `teshi-id` attribute assigned

#### Scenario: Inputs receive teshi-id
- **WHEN** a page loads with an `<input type="text">` element
- **THEN** the input SHALL have a `teshi-id` attribute assigned

### Requirement: Ref → element descriptor mapping
The system SHALL maintain a mapping table that associates each `teshi-id` with its element role, accessible name, tag, and element type.

#### Scenario: Structured snapshot returns element list
- **WHEN** the agent calls `get_structured_snapshot`
- **THEN** the response SHALL contain a list of all interactive elements with their teshi-id, role, name, tag, and type

### Requirement: Page navigation injection
The injection SHALL run automatically on every page navigation and new document.

#### Scenario: Injection persists across navigations
- **WHEN** the browser navigates to a new page
- **THEN** the injection script SHALL execute on the new page
