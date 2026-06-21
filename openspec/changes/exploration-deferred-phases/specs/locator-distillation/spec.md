## ADDED Requirements

### Requirement: Multi-dimensional feature capture
During exploration, the sidecar SHALL record per-element multi-dimensional features alongside the `teshi-id`: DOM path, CSS selector candidates, accessibility role, accessible name, text content, XPath, bounding box, and element tag name.

#### Scenario: Features captured during click
- **WHEN** the agent calls `browser_click(ref="e15")`
- **THEN** the sidecar SHALL record all dimension features for the element with `teshi-id="e15"` at that moment

### Requirement: Locator stability scoring
The distillation engine SHALL score each candidate locator by stability criteria: presence of data-testid (highest), usage of static id/name, semantic role+name pairing, reliance on DOM structure, and presence of dynamic class hashes (lower score).

#### Scenario: data-testid locator scores highest
- **WHEN** the engine evaluates an element that has a `data-testid` attribute
- **THEN** `page.getByTestId(...)` SHALL be ranked as the most stable locator

#### Scenario: Class-based locator is demoted
- **WHEN** the engine evaluates an element whose CSS class contains a dynamic hash pattern (e.g., `css-1a2b3c`)
- **THEN** class-based locators SHALL receive a lower stability score

### Requirement: Locator ranking output
The distillation engine SHALL output a ranked list of locator candidates for each traced action, ordered from most to least stable.

#### Scenario: Engine produces ranked locators
- **WHEN** the engine processes a trace
- **THEN** each action in the output SHALL have an ordered list of at least 3 candidate locators with their stability scores
