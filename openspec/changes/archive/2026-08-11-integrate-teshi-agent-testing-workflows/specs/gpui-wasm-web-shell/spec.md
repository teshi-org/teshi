## Purpose

Defines the supported GPUI WASM web surface for selecting browser-extension sessions and removes the retired React application.

## ADDED Requirements

### Requirement: Official web surface is GPUI WASM
`teshi web` SHALL serve the distribution built from `apps/teshi-web`. The repository SHALL NOT retain `apps/teshi-web-ui`, and supported development, installer, and release workflows SHALL NOT resolve, build, test, or package the retired React application.

#### Scenario: User starts teshi web
- **WHEN** a user starts `teshi web` without an explicit `--dist`
- **THEN** the daemon SHALL resolve the GPUI WASM distribution and the loaded application SHALL identify itself as the GPUI web shell

#### Scenario: Release package is assembled
- **WHEN** an installer or release archive advertising browser locator support is built
- **THEN** its bundled `share/web` assets SHALL originate from `apps/teshi-web/dist` and SHALL NOT contain the React application bundle

#### Scenario: Repository structure is inspected
- **WHEN** the application directories and frontend tooling are inspected
- **THEN** `apps/teshi-web-ui` and its React package metadata, source, tests, and build configuration SHALL NOT exist

### Requirement: Shared GPUI browser-session view
The browser-session view SHALL be implemented in the shared GPUI UI crate and used by both the native Desktop shell and GPUI WASM Web shell.

#### Scenario: Multiple profiles are available
- **WHEN** two or more eligible extension sessions are listed and the user has not explicitly selected one
- **THEN** the GPUI view SHALL remain unselected, explain that a profile must be chosen, and SHALL NOT show another profile's tabs or frame as selected

#### Scenario: Exactly one profile is available
- **WHEN** exactly one eligible extension session is listed and no previous explicit selection exists
- **THEN** the view MAY select that session for single-profile compatibility and SHALL show its opaque identity and health

#### Scenario: Selected profile disconnects
- **WHEN** an explicitly selected extension session expires or disconnects while another session remains live
- **THEN** the view SHALL retain the missing selection as unavailable and SHALL NOT silently switch to the other profile

### Requirement: Same-origin broker adapter
The GPUI WASM application SHALL discover sessions and activate selected tabs through narrow same-origin daemon endpoints while the extension broker remains loopback-only.

#### Scenario: Web shell is served through a non-loopback daemon address
- **WHEN** the GPUI WASM page requests browser-session inventory or tab activation
- **THEN** it SHALL call its own daemon origin and the daemon SHALL proxy only the corresponding loopback broker operation

#### Scenario: Broker is unavailable
- **WHEN** the loopback browser broker is not running or cannot be reached
- **THEN** the daemon SHALL return an actionable non-success response and the GPUI view SHALL display the unavailable state without selecting or mutating a browser session
