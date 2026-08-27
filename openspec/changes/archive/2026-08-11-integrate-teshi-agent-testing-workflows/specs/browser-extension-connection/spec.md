## Purpose

Defines how a browser-profile Teshi extension is installed, identified, registered, and diagnosed so external agents can inspect a real local browser session safely.

## ADDED Requirements

### Requirement: Installable browser extension
Teshi SHALL provide a compatible Chromium extension bundle and installation guidance as part of releases that advertise browser locator acquisition.

#### Scenario: User prepares a browser profile
- **WHEN** a user installs the Teshi extension into a supported Chromium profile and starts the local broker
- **THEN** the extension SHALL report a connected state and the broker SHALL expose actionable compatibility and health information

### Requirement: Stable extension instance identity
Each browser-profile extension installation SHALL generate and persist an opaque instance identifier in profile-local extension storage and SHALL include it in every registration, heartbeat, command response, and frame message.

#### Scenario: Extension service worker restarts
- **WHEN** the extension service worker restarts within the same browser profile
- **THEN** it SHALL reconnect using the same instance identifier

#### Scenario: Two profiles install the extension
- **WHEN** two browser profiles run separate Teshi extension installations
- **THEN** the broker SHALL register them as distinct instances even if their extension version and visible labels are identical

### Requirement: User-facing profile label
The extension SHALL allow an optional user-facing label for its browser-profile instance while keeping the opaque instance identifier as the routing key.

#### Scenario: Labels are duplicated
- **WHEN** two registered instances use the same label
- **THEN** Teshi SHALL display both as distinct sessions and SHALL NOT route a command by label alone

### Requirement: Local-only broker connection
The extension SHALL connect to a Teshi broker on loopback by default and SHALL NOT make browser control available to unrelated remote hosts.

#### Scenario: Broker is unavailable
- **WHEN** the extension cannot reach the configured local broker
- **THEN** its UI SHALL report the disconnected state and provide actionable local startup or compatibility guidance

### Requirement: Protocol compatibility preflight
The extension and broker SHALL exchange protocol and component versions before the broker permits debugger attachment or mutating commands.

#### Scenario: Extension protocol is incompatible
- **WHEN** a registered extension uses an unsupported protocol version
- **THEN** Teshi SHALL mark the session incompatible and SHALL reject locator mutation with detected and required version information

### Requirement: Bounded registration lifecycle
The broker SHALL expire extension instances that stop heartbeating and SHALL bound per-instance command and response queues.

#### Scenario: Browser profile closes
- **WHEN** an extension instance stops heartbeating beyond the configured expiry
- **THEN** the broker SHALL mark the session disconnected, cancel or fail pending requests, and make any lease recoverable
