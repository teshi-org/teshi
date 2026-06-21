## ADDED Requirements

### Requirement: Interactive element identification
The system SHALL traverse the DOM at page load and inject a `[teshi-id]` attribute on every interactive element (buttons, inputs, links, selects, textareas, and elements with `tabindex` or explicit `role` attributes).

#### Scenario: Buttons receive teshi-id
- **WHEN** a page containing `<button>Sign In</button>` loads
- **THEN** the button element SHALL have a `teshi-id` attribute with a unique value

#### Scenario: Input fields receive teshi-id
- **WHEN** a page containing `<input type="text" name="email" />` loads
- **THEN** the input element SHALL have a `teshi-id` attribute with a unique value

#### Scenario: Links receive teshi-id
- **WHEN** a page containing `<a href="/login">Login</a>` loads
- **THEN** the anchor element SHALL have a `teshi-id` attribute with a unique value

### Requirement: Non-interactive elements are skipped
The system SHALL NOT inject `[teshi-id]` on non-interactive elements (div, span, p, h1-h6, section, aside, etc.) unless they have a `tabindex` or explicit `role`.

#### Scenario: Plain div is skipped
- **WHEN** a page contains `<div class="container">content</div>`
- **THEN** the div SHALL NOT receive a `teshi-id` attribute

### Requirement: Ref mapping table
The system SHALL maintain a mapping from `teshi-id` to the element's multi-dimensional descriptor, accessible to the agent via the snapshot tool.

#### Scenario: Agent retrieves ref descriptor
- **WHEN** the agent calls `browser_snapshot` after page load
- **THEN** each interactive element in the snapshot SHALL include its `teshi-id`, role, accessible name, and element type
