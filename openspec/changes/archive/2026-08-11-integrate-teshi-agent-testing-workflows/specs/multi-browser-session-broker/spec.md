## Purpose

Defines concurrent discovery, targeting, ownership, and isolation for multiple Teshi browser-extension instances and the agents that use them.

## ADDED Requirements

### Requirement: Multi-instance session registry
The Teshi browser broker SHALL maintain independent live session records for multiple registered extension instances.

#### Scenario: Multiple profiles connect
- **WHEN** extensions in two or more browser profiles heartbeat concurrently
- **THEN** session discovery SHALL return each live instance with its opaque identity, display label, health, browser metadata, windows, and eligible tabs

### Requirement: Composite explicit browser target
Browser operations SHALL use an extension instance identifier, window identifier, and tab identifier as the canonical target and SHALL correlate every command and response with a unique request identifier.

#### Scenario: Tab identifiers collide across profiles
- **WHEN** two extension instances report the same numeric tab identifier
- **THEN** commands SHALL remain scoped to the selected extension instance and SHALL NOT be delivered to the other tab

#### Scenario: Response target does not match
- **WHEN** the broker receives a response whose instance, target, or request identifier does not match a pending request
- **THEN** it SHALL reject or quarantine the response and SHALL NOT deliver it to another agent

### Requirement: Ambiguous implicit targeting fails closed
Legacy implicit browser targeting SHALL resolve a target only when exactly one eligible session and tab exist and SHALL fail closed otherwise.

#### Scenario: More than one browser session is eligible
- **WHEN** an agent invokes a browser operation without a target and multiple sessions are available
- **THEN** Teshi SHALL return an `ambiguous_browser_target` error with non-sensitive selection metadata and SHALL perform no browser action

### Requirement: Exclusive mutation lease
Teshi SHALL require a valid exclusive session lease for navigation, highlighting, locator acquisition, debugger attachment, clicks, typing, and other operations that depend on mutable browser state.

#### Scenario: Agent reserves an available session
- **WHEN** an agent acquires a lease for an unleased live session
- **THEN** Teshi SHALL return an opaque lease token and expiry and SHALL accept matching targeted operations until release or expiry

#### Scenario: Another agent requests a leased session
- **WHEN** a second agent requests an exclusive lease held by a live first agent
- **THEN** Teshi SHALL reject the acquisition with non-sensitive owner and expiry information and SHALL NOT disturb the first agent's work

### Requirement: Lease recovery
Leases SHALL have a renewable bounded lifetime and SHALL become recoverable after release, owner disconnect, or expiry.

#### Scenario: Agent crashes while holding a lease
- **WHEN** the lease is not renewed before its expiry
- **THEN** Teshi SHALL cancel or bound in-flight work, release debugger ownership where possible, and make the session available again

### Requirement: Parallel session isolation
Teshi SHALL permit different agents to operate different leased browser sessions concurrently without sharing command queues, responses, frames, or mutable target state.

#### Scenario: Two agents acquire different profiles
- **WHEN** Agent A leases Profile A and Agent B leases Profile B
- **THEN** both agents SHALL be able to acquire snapshots and locators concurrently and each result SHALL identify only its selected target
