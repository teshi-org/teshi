## ADDED Requirements

### Requirement: teshi web launches an ephemeral loopback session
Production `teshi web` SHALL start or select its daemon on `127.0.0.1` using an OS-selected available port when no diagnostic port is supplied, SHALL mint a cryptographically random in-memory hosted-UI session token, and SHALL NOT require a local Web distribution.

#### Scenario: User starts teshi web with default options
- **WHEN** no port override is supplied
- **THEN** the launcher SHALL use an available dynamic loopback port rather than defaulting to fixed port `20253`
- **AND** the daemon session SHALL exist before the browser is opened

#### Scenario: Local Web assets are absent
- **WHEN** neither `apps/teshi-web/dist` nor installed `share/web` exists
- **THEN** production `teshi web` SHALL still start the daemon and open the hosted application

### Requirement: Launch data stays in the URL fragment
The launcher SHALL open `https://teshi-org.github.io/app/#port=<port>&token=<session-token>`, and the hosted application SHALL consume the port and token from the fragment without sending them in the initial HTTP request. The compatible custom-domain entrypoint `https://teshi.org/app/` SHALL remain supported.

#### Scenario: Hosted page is requested
- **WHEN** the system browser navigates to the launch URL
- **THEN** the HTTP request target, server logs, query string, and referrer data SHALL contain neither the daemon port nor the session token

#### Scenario: Hosted application consumes launch data
- **WHEN** the application has copied valid launch data into memory
- **THEN** it SHALL remove the fragment from the visible history entry with `history.replaceState`
- **AND** it SHALL NOT persist the token in local storage, session storage, IndexedDB, cache data, or logs

### Requirement: Hosted sessions are ephemeral and scoped
A hosted-UI session SHALL be stored only in daemon memory, SHALL be scoped to the launcher-created daemon session, and SHALL become invalid on teardown, daemon exit, or daemon restart.

#### Scenario: Daemon restarts
- **WHEN** a hosted page attempts to reconnect with a token minted by the previous daemon process
- **THEN** the new daemon SHALL reject that token without executing a business operation

#### Scenario: Socket reconnects during the same session
- **WHEN** the same hosted page reconnects after a transient disconnect while the daemon session remains valid
- **THEN** it SHALL be able to repeat the first-message handshake with the in-memory token
- **AND** non-idempotent requests from the lost connection SHALL NOT be replayed automatically

### Requirement: WebSocket upgrades require the trusted hosted Origin
Production `/ws/control` and `/ws/preview` upgrades SHALL accept the exact `Origin` value `https://teshi.org` or `https://teshi-org.github.io` and SHALL reject every other browser Origin before any sidecar connection or business processing.

#### Scenario: Trusted hosted UI upgrades
- **WHEN** a WebSocket upgrade carries `Origin: https://teshi.org` or `Origin: https://teshi-org.github.io`
- **THEN** the daemon SHALL allow the upgrade to proceed to application-level authentication

#### Scenario: Untrusted or missing Origin upgrades
- **WHEN** the Origin is missing, malformed, `null`, non-HTTPS, a subdomain, a lookalike domain, or any value other than the configured exact production Origin
- **THEN** the daemon SHALL reject the production browser upgrade before WebSocket establishment

### Requirement: First message authenticates and negotiates the channel
The first application message on each WebSocket SHALL be a versioned `client_hello` containing the session token, requested channel, protocol version, and UI compatibility identity. No RPC, event subscription, preview attachment, or other business action SHALL occur before validation succeeds.

#### Scenario: Valid control handshake
- **WHEN** `/ws/control` receives a valid first-message hello for a live hosted-UI session and compatible protocol
- **THEN** the daemon SHALL reply with `server_hello` containing its build identity, selected protocols, session identity, and granted capabilities

#### Scenario: First message is invalid
- **WHEN** the first message times out, is malformed, uses the wrong channel, supplies an invalid or expired token, or has no compatible protocol
- **THEN** the daemon SHALL send or close with a stable machine-readable reason
- **AND** it SHALL perform no business operation

### Requirement: Hosted UI never receives implicit Admin
The daemon SHALL authorize a valid hosted client with a dedicated `HostedWebUi` role or equivalent explicit capability set and SHALL NOT infer Admin from a loopback TCP peer.

#### Scenario: Tokenless loopback client connects
- **WHEN** a browser WebSocket from a loopback peer supplies no valid hosted session token
- **THEN** it SHALL receive no business capability and SHALL NOT invoke an Admin operation

#### Scenario: Authenticated hosted client requests an ungranted operation
- **WHEN** a valid hosted session requests a method outside its enumerated capability set
- **THEN** the daemon SHALL reject that request with a forbidden error without invoking its handler
