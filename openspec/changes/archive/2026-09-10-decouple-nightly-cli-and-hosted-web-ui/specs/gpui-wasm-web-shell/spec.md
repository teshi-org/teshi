## MODIFIED Requirements

### Requirement: Official web surface is GPUI WASM
The official Web UI SHALL be the distribution built from `apps/teshi-web` and hosted at `https://teshi.org/app/`. Production `teshi web` SHALL open that hosted application and SHALL NOT require the daemon to resolve, serve, or package a local Web distribution. The repository SHALL NOT retain or restore the retired React application.

#### Scenario: User starts teshi web
- **WHEN** a user starts production `teshi web`
- **THEN** the loaded application SHALL come from `https://teshi.org/app/` and SHALL identify itself as the GPUI WASM shell
- **AND** startup SHALL succeed without `apps/teshi-web/dist`, installed `share/web`, or `--dist`

#### Scenario: Release package is assembled
- **WHEN** an installer or release archive advertising browser locator support is built
- **THEN** its nightly payload SHALL NOT contain `apps/teshi-web/dist`, `share/web`, or another complete Web UI bundle

#### Scenario: Nightly package is assembled
- **WHEN** a nightly installer or release payload is built
- **THEN** it SHALL NOT contain `apps/teshi-web/dist`, `share/web`, or another complete Web UI bundle

#### Scenario: Repository structure is inspected
- **WHEN** application directories and frontend tooling are inspected
- **THEN** `apps/teshi-web` SHALL remain the GPUI WASM source shared with native UI code
- **AND** `apps/teshi-web-ui` and its React package metadata, source, tests, and build configuration SHALL NOT exist

### Requirement: Same-origin broker adapter
The hosted GPUI WASM application SHALL discover sessions and activate selected tabs through authenticated methods on its negotiated loopback `/ws/control` connection, while the extension broker remains loopback-only and undisclosed to the page.

#### Scenario: Web shell is served through a non-loopback daemon address
- **WHEN** the GPUI WASM page requests browser-session inventory or tab activation
- **THEN** it SHALL use the authenticated daemon control connection and the daemon SHALL proxy only the corresponding loopback broker operation

#### Scenario: Hosted shell requests browser-session inventory
- **WHEN** the authenticated hosted UI lists browser sessions or activates an explicit tab
- **THEN** it SHALL use the corresponding control RPC and the daemon SHALL proxy only the narrow loopback broker operation

#### Scenario: Broker is unavailable
- **WHEN** the loopback browser broker is not running or cannot be reached
- **THEN** the daemon SHALL return an actionable RPC error and the GPUI view SHALL display the unavailable state without selecting or mutating a browser session

## ADDED Requirements

### Requirement: Hosted shell uses daemon coordinates only from launch state
The hosted GPUI WASM application SHALL derive `ws://127.0.0.1:<port>/ws/control` and `ws://127.0.0.1:<port>/ws/preview` from validated in-memory launch state and SHALL NOT derive local daemon endpoints from the hosted page authority.

#### Scenario: Valid launch fragment is parsed
- **WHEN** `/app/` receives a valid loopback port and token in its fragment
- **THEN** it SHALL connect directly to the corresponding loopback daemon endpoints and SHALL send application traffic neither to the Hugo origin nor through a remote relay

#### Scenario: Launch data is absent or invalid
- **WHEN** the hosted page is opened without a valid port/token pair
- **THEN** it SHALL show launch guidance and SHALL NOT scan loopback ports or attempt unauthenticated business connections
