## MODIFIED Requirements

### Requirement: Local-only broker connection
The extension SHALL connect to the Rust Teshi browser broker on loopback by default and SHALL NOT make browser control available to unrelated remote hosts. Chrome extension mode SHALL start and operate without Python, pip, uv, a project virtual environment, or Playwright. Credential delivery SHALL be restricted to an explicit, per-user allowlist of at most 16 exact Teshi extension origins; ordinary web-page origins and unpaired extensions SHALL NOT receive broker credentials or read discovery data. The allowlist SHALL change only through an explicit pairing or removal action and SHALL NOT silently migrate when an extension ID changes.

#### Scenario: Broker is unavailable
- **WHEN** the extension cannot reach the configured local broker
- **THEN** its UI SHALL report the disconnected state and provide actionable local startup or compatibility guidance

#### Scenario: Python is absent
- **WHEN** a user starts Chrome extension automation on a system with no Python installation or project virtual environment
- **THEN** Teshi SHALL start or reuse the Rust broker and the extension SHALL complete registration without running Python tooling

#### Scenario: An untrusted origin requests discovery
- **WHEN** an ordinary web-page origin or unsupported extension origin requests broker discovery
- **THEN** the broker SHALL deny browser-readable discovery, SHALL NOT return its bearer credential, and SHALL accept no mutation from that origin

#### Scenario: A local listener spoofs broker metadata
- **WHEN** a local listener reports compatible public discovery fields but cannot produce a fresh proof for the private broker credential
- **THEN** Teshi SHALL reject it as the current broker, SHALL NOT send the bearer token to it, and SHALL leave the listener untouched

#### Scenario: A supported install channel has a different unpacked ID
- **WHEN** the user explicitly pairs the exact ID shown by another Teshi unpacked install channel
- **THEN** only that exact origin SHALL receive the credential, and already paired IDs SHALL remain authorized

#### Scenario: An unpacked extension folder moves
- **WHEN** moving an unpacked extension changes its Chrome ID
- **THEN** the new ID SHALL remain unpaired until explicit user action, the old ID SHALL remain unchanged, and the user SHALL be able to verify existing Profile storage before removing the old ID
