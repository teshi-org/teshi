## ADDED Requirements

### Requirement: Per-session reference and diagnostic isolation
The broker SHALL isolate element-reference caches, artifact metadata, console/network buffers, operation diffs, and privileged grant state by extension instance and complete target.

#### Scenario: Reference aliases collide across profiles
- **WHEN** two profiles each expose an element reference named `@e1`
- **THEN** the broker SHALL resolve each alias only within its recorded session, target, snapshot, and page revision

### Requirement: Expanded lease enforcement
The broker SHALL require a valid exclusive profile lease for element-reference resolution, pointer or keyboard input, tab/window mutation, artifact capture, upload, diagnostic attachment, and every privileged browser operation.

#### Scenario: Unleased caller requests a screenshot
- **WHEN** a caller requests a target-scoped screenshot without the selected profile lease
- **THEN** the broker SHALL reject the request before debugger attachment or artifact creation

### Requirement: At-most-once mutating request dispatch
The broker SHALL prevent retry, fallback transport, timeout recovery, or duplicate request identifiers from dispatching the same mutating operation more than once.

#### Scenario: Command response is delayed during fallback
- **WHEN** a transport fallback occurs after a mutation was already accepted
- **THEN** the broker SHALL correlate the original request and SHALL NOT enqueue a second mutation

### Requirement: Capability grants are not discovery data
Session discovery SHALL advertise available capability names and public availability states but SHALL NOT expose capability grant tokens, lease tokens, Cookie values, or privileged result content.

#### Scenario: Agent lists sessions during privileged work
- **WHEN** another agent lists a profile with an active privileged grant
- **THEN** discovery SHALL reveal only non-sensitive busy and capability availability metadata
