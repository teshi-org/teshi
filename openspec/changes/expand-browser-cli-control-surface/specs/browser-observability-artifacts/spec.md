## ADDED Requirements

### Requirement: Target-scoped screenshots
Teshi SHALL capture viewport, full-page, and element screenshots for an explicitly targeted leased tab and SHALL write binary image data to a requested or managed file rather than normal JSON output.

#### Scenario: Agent captures an element screenshot
- **WHEN** an agent supplies a current reference or structured locator and an output path
- **THEN** Teshi SHALL verify the element, capture only its rendered bounds, and return path, size, format, target, request ID, and page revision

#### Scenario: Element is below the current viewport origin
- **WHEN** an element screenshot is requested after the page or an inspected frame has scrolled
- **THEN** Teshi SHALL convert rendered viewport bounds to CDP page coordinates before capture

### Requirement: Target-scoped PDF output
Teshi SHALL support bounded PDF generation with explicit paper, orientation, scale, and background options on backends that advertise PDF capability.

#### Scenario: Backend cannot print PDF
- **WHEN** a selected backend does not advertise PDF support
- **THEN** Teshi SHALL return `browser_capability_unavailable` without creating a partial artifact

### Requirement: Bounded console capture
Teshi SHALL provide start, list, clear, and stop operations for per-session console capture using bounded age, entry-count, and byte retention.

#### Scenario: Two profiles capture console events
- **WHEN** console capture is active for two leased profiles
- **THEN** each list operation SHALL return only events correlated to its selected profile and target

### Requirement: Bounded network capture
Teshi SHALL provide start, list, detail, clear, and stop operations for per-session network capture with metadata-only defaults, explicit bounded body retrieval, and truncation markers.

#### Scenario: Caller requests a large response body
- **WHEN** a captured body exceeds the configured byte limit
- **THEN** Teshi SHALL return a truncated result with encoding and original-size metadata and SHALL NOT emit the full body

### Requirement: Sensitive diagnostic redaction
Teshi SHALL redact authorization, Cookie, token, password, and configured sensitive headers or fields from console/network summaries and audit metadata by default.

#### Scenario: Request contains an Authorization header
- **WHEN** the agent lists captured network requests
- **THEN** Teshi SHALL preserve the header name while replacing its value with a redaction marker

### Requirement: Operation before and after summaries
Browser mutations SHALL optionally capture bounded before/after page summaries and return a structured difference without repeating the mutation.

#### Scenario: Click changes visible text
- **WHEN** monitoring is requested and a click completes
- **THEN** Teshi SHALL return a bounded summary of relevant added, removed, or changed page state tied to the action request

### Requirement: Explicit file upload
Teshi SHALL upload only caller-specified local files to an explicitly targeted file input or drop target and SHALL validate target, lease, file existence, size policy, and actionability before mutation.

#### Scenario: Upload path is missing or disallowed
- **WHEN** the supplied path does not exist or violates the active filesystem policy
- **THEN** Teshi SHALL fail before reading page state or mutating the input

### Requirement: Artifact lifecycle and cleanup
Teshi SHALL correlate artifacts and diagnostic buffers to project, target, request, and creation time and SHALL provide bounded cleanup without deleting user-selected output files implicitly.

#### Scenario: Broker stops
- **WHEN** the broker stops or a profile disconnects
- **THEN** Teshi SHALL clear in-memory console, network, reference, and diff caches while preserving documented artifact files

### Requirement: Compatible bounded artifact transport
The extension, HTTP fallback, and WebSocket command transport SHALL enforce compatible bounds large enough for an allowed encoded artifact and SHALL reject oversized artifacts before sending or persisting them.

#### Scenario: Encoded artifact exceeds the managed limit
- **WHEN** screenshot or PDF data exceeds the documented managed artifact limit
- **THEN** Teshi SHALL return `browser_artifact_failure` without sending a payload the broker transport cannot accept and without creating a partial file
