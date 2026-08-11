## Purpose

Defines how Teshi inspects a selected browser page and returns ranked, verified locator candidates that an external agent can use while authoring Playwright automation.

## ADDED Requirements

### Requirement: Structured page inspection
Teshi SHALL acquire a structured snapshot from an explicitly targeted browser tab containing the accessible and interactive element information needed for locator selection.

#### Scenario: Agent inspects a signed-in page
- **WHEN** an agent with a valid session lease requests a snapshot for a selected tab
- **THEN** Teshi SHALL return the page URL, title, target identity, page-context revision, and structured element data without exposing cookies or browser-profile filesystem data

### Requirement: Ranked Playwright locator candidates
Teshi SHALL return ranked Playwright locator candidates with structured locator arguments and a rendered Playwright expression.

#### Scenario: Element has a unique accessible role and name
- **WHEN** the intended element can be uniquely selected by role and accessible name
- **THEN** Teshi SHALL prefer a `getByRole` candidate over a generated class, long DOM path, positional selector, or coordinate

#### Scenario: Semantic locator is unavailable
- **WHEN** no supported semantic locator uniquely identifies the intended element
- **THEN** Teshi MAY return a stable attribute or CSS fallback and SHALL identify its stability limitations

### Requirement: Locator quality metadata
Each locator candidate SHALL include its kind, arguments, rendered expression, match count, frame or shadow context where applicable, verification status, stability rationale, and warnings.

#### Scenario: Candidate matches multiple elements
- **WHEN** a generated candidate is not unique in the targeted page context
- **THEN** Teshi SHALL mark it ambiguous and SHALL NOT present it as the verified recommendation

### Requirement: In-browser locator verification
Teshi SHALL evaluate a recommended locator in the targeted browser context before reporting it as verified.

#### Scenario: Recommended locator is verified
- **WHEN** the locator resolves to the intended visible or actionable element in the selected tab and frame
- **THEN** the result SHALL report a successful verification and the observed match count

#### Scenario: Page changes during acquisition
- **WHEN** navigation or document replacement invalidates the snapshot before verification completes
- **THEN** Teshi SHALL return a stale-page-context error or an explicitly unverified result rather than claiming success

### Requirement: Configurable test-id attributes
Teshi SHALL support project-configured test-id attribute names while retaining documented defaults.

#### Scenario: Project defines a custom test-id attribute
- **WHEN** a unique intended element exposes that configured attribute
- **THEN** Teshi SHALL be able to render and verify the corresponding Playwright test-id locator

### Requirement: Locator acquisition does not invent test behavior
Teshi SHALL limit locator acquisition to inspection, target disambiguation, locator rendering, and verification and SHALL NOT invent navigation, input values, assertions, or destructive actions not requested by the caller.

#### Scenario: Agent asks for a submit-button locator
- **WHEN** Teshi resolves and verifies the requested element
- **THEN** it SHALL return locator candidates without clicking the button unless the caller separately requests an authorized execution operation
