# Requirements-to-Testpoints Page

## Purpose

Provide a dedicated page where users can paste free-text requirements, generate test points as a FreeMind mindmap, preview a high-fidelity mock HTML page, and navigate bidirectionally between test points and requirement words.

## Requirements

### Requirement: User inputs free-text requirements

The system SHALL provide a text input area where the user can paste or type free-text requirements. The input area SHALL support multi-line text and retain line breaks.

#### Scenario: User pastes requirement text

- **WHEN** user pastes text into the requirements input area
- **THEN** the text is displayed in the input area preserving original formatting

#### Scenario: Empty input is rejected

- **WHEN** user clicks "Generate" with empty input
- **THEN** the system SHALL display a warning message and not proceed

---

### Requirement: One-click AI generation

The system SHALL provide a single "Generate" button that initiates the full AI pipeline: requirement segmentation, test point mindmap generation, and mock HTML generation. The system SHALL display a loading indicator during generation.

#### Scenario: User triggers generation

- **WHEN** user clicks the "Generate" button after entering requirements text
- **THEN** the system SHALL send the text to the backend, display a loading state, and render results upon completion

#### Scenario: Generation failure handling

- **WHEN** the AI generation fails (network error, LLM error, or malformed response)
- **THEN** the system SHALL display an error message and allow the user to retry

---

### Requirement: AI splits requirements into word-level segments

The system SHALL instruct the LLM to split the free-text requirements into word-level segments. Each segment SHALL have a unique ID, text content, and character position range.

#### Scenario: Requirements are split into word segments

- **WHEN** the user submits requirements text for generation
- **THEN** the LLM response SHALL include a `segments` array where each segment has `id`, `text`, and `pos` fields

#### Scenario: Segments cover the entire input

- **WHEN** segmentation is complete
- **THEN** every character in the input text SHALL be covered by at least one segment, with no gaps or overlaps

---

### Requirement: AI generates test points as FreeMind mindmap

The system SHALL instruct the LLM to generate test points as a FreeMind-compatible XML document (.mm format). Each test point node SHALL be a leaf node in the tree. Each leaf node SHALL include a `LINK` attribute referencing one or more requirement word segment IDs.

#### Scenario: FreeMind XML is valid

- **WHEN** the LLM returns a `mindmap_xml` field
- **THEN** the XML SHALL be well-formed and parseable as a FreeMind document

#### Scenario: Test point nodes link to requirement segments

- **WHEN** a test point is generated
- **THEN** its `LINK` attribute SHALL contain comma-separated word segment IDs (e.g., `LINK="w3,w4"`)

#### Scenario: Mindmap is rendered as a tree

- **WHEN** the FreeMind XML is loaded
- **THEN** the system SHALL render it as an interactive tree view with expandable/collapsible nodes

---

### Requirement: AI generates high-fidelity mock HTML

The system SHALL instruct the LLM to generate a high-fidelity mock HTML page that demonstrates the user interface logic described by the requirements. The HTML SHALL include realistic form elements, layout, and styling.

#### Scenario: Mock HTML is generated

- **WHEN** the LLM returns a `mock_html` field
- **THEN** the HTML SHALL be a complete, self-contained HTML document

#### Scenario: Mock HTML is rendered in a sandboxed iframe

- **WHEN** the mock HTML is loaded
- **THEN** the system SHALL render it inside a sandboxed iframe with no access to the parent page

---

### Requirement: Bidirectional mapping between test points and requirement words

The system SHALL support bidirectional navigation between test point nodes and requirement word segments. Clicking a test point node in the mindmap SHALL highlight the corresponding words in the requirements text. Clicking highlighted words in the requirements text SHALL highlight the corresponding test point nodes.

#### Scenario: Clicking test point highlights requirement words

- **WHEN** user clicks a test point node that has `LINK="w3,w4"`
- **THEN** the words with segment IDs `w3` and `w4` in the requirements text SHALL be visually highlighted

#### Scenario: Clicking requirement words highlights test point

- **WHEN** user clicks a highlighted word segment in the requirements text
- **THEN** the corresponding test point nodes in the mindmap SHALL be visually highlighted and scrolled into view

#### Scenario: Mock HTML updates on test point selection

- **WHEN** user clicks a test point node
- **THEN** the mock HTML preview SHALL scroll to or highlight the section corresponding to the selected test point

---

### Requirement: Requirements-to-testpoints page as default view

The system SHALL display the requirements-to-testpoints page as the default view when the application starts (in both desktop and web modes). The page SHALL contain a three-panel layout: requirements input (left), test point mindmap (center), and mock HTML preview (right).

#### Scenario: Page is the default view on startup

- **WHEN** the desktop or web app is launched
- **THEN** the requirements-to-testpoints page SHALL be the first visible view

---

### Requirement: One-click toggle to original workspace

The system SHALL provide a toggle button fixed at the top-left corner of the application. Clicking this button SHALL instantaneously switch between the requirements-to-testpoints page and the original workspace (Gherkin editor, screencast, terminal, bottom dock).

#### Scenario: Toggle button switches views instantly

- **WHEN** user clicks the toggle button
- **THEN** the view SHALL switch between the requirements page and the original workspace with no animation delay or loading

#### Scenario: Toggle button is always visible

- **WHEN** the application is running in any view
- **THEN** the toggle button SHALL remain visible and fixed at the top-left corner

---

### Requirement: Generated results persistence

The system SHALL persist generated results to the `.teshi/testpoints/<slug>/` directory within the project. The slug SHALL be derived from the generation timestamp or a user-provided name. The directory SHALL contain `requirements.mm` (the FreeMind mindmap) and `mock.html` (the mock HTML).

#### Scenario: Results are saved on generation

- **WHEN** generation completes successfully
- **THEN** the `.teshi/testpoints/<slug>/` directory SHALL be created with `requirements.mm` and `mock.html`

#### Scenario: Saved results can be reloaded

- **WHEN** the page loads and existing testpoints are found
- **THEN** the most recent testpoints SHALL be loaded and rendered automatically
