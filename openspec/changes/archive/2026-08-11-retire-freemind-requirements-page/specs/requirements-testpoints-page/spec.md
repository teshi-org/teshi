## REMOVED Requirements

### Requirement: User inputs free-text requirements

**Reason**: Requirements input moves to the TUI Agent pipeline; desktop/web no longer host a requirements page.
**Migration**: Paste or describe requirements in the TUI AI chat and follow the generation pipeline (`submit_requirements` → plan → write features).

### Requirement: One-click AI generation

**Reason**: FreeMind/mock HTML one-shot generation API is retired.
**Migration**: Use TUI Agent generation to produce Gherkin scenarios.

### Requirement: AI splits requirements into word-level segments

**Reason**: Segment IDs existed only to link FreeMind leaf nodes to requirement words; FreeMind is retired.
**Migration**: None; Gherkin scenarios replace linked test-point nodes.

### Requirement: AI generates test points as FreeMind mindmap

**Reason**: FreeMind `.mm` format is no longer used.
**Migration**: Test points are Gherkin scenarios/steps generated in the TUI.

### Requirement: AI generates high-fidelity mock HTML

**Reason**: Mock HTML preview was part of the desktop/web Requirements page and has no TUI surface.
**Migration**: None; do not generate or persist `mock.html` for this flow.

### Requirement: Bidirectional mapping between test points and requirement words

**Reason**: Mapping depended on FreeMind `LINK` attributes and the Requirements page UI.
**Migration**: Review scenarios in the TUI Gherkin MindMap / Explore views.

### Requirement: Requirements-to-testpoints page as default view

**Reason**: Desktop/web default view is Workspace; generation is TUI-owned.
**Migration**: Open the app to Workspace; use TUI for generation.

### Requirement: One-click toggle to original workspace

**Reason**: Requirements mode and toggle are removed; only Workspace remains.
**Migration**: None.

### Requirement: Generated results persistence

**Reason**: `.teshi/testpoints/<slug>/requirements.mm` and `mock.html` persistence is retired.
**Migration**: Generated scenarios persist as `.feature` files under the project; historical `testpoints` dirs are unsupported.

## ADDED Requirements

### Requirement: GUI does not provide requirements-to-testpoints generation

The desktop and web applications SHALL NOT present a requirements-to-testpoints generation page, FreeMind mindmap editor for test points, mock HTML generation UI, or an API endpoint that generates FreeMind/mock HTML testpoint artifacts. On startup, the application SHALL show the Workspace view (project welcome or editor workspace) without a Requirements/Workspace mode toggle.

#### Scenario: Startup shows Workspace

- **WHEN** the desktop or web app is launched
- **THEN** the Requirements-to-testpoints page SHALL NOT be shown as the default view

#### Scenario: Generate API is unavailable

- **WHEN** a client calls the former requirements generate endpoint (or equivalent Tauri command)
- **THEN** the system SHALL NOT accept and complete FreeMind/mock HTML testpoint generation
