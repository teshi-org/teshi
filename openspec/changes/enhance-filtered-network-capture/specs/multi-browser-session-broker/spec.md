## MODIFIED Requirements

### Requirement: Parallel session isolation
Teshi SHALL permit different agents to operate different leased browser sessions concurrently without sharing command queues, responses, frames, network capture identifiers, network events, request bodies, acknowledgements, or mutable target state.

#### Scenario: Two agents acquire different profiles
- **WHEN** Agent A leases Profile A and Agent B leases Profile B
- **THEN** both agents SHALL be able to acquire snapshots, locators, and hostname-filtered network captures concurrently and each result SHALL identify only its selected target and capture

#### Scenario: Captures reuse a CDP request identifier
- **WHEN** different Profiles report the same CDP network request identifier
- **THEN** the broker SHALL retain separate request records under their Profile, target, and capture identities

## ADDED Requirements

### Requirement: Concurrent tab captures under one Profile lease
One valid Profile lease SHALL permit its owner to run independent captures on multiple explicit tabs in that Profile without one tab's preview or debugger lifecycle silently stopping another tab's capture.

#### Scenario: Lease owner captures two tabs
- **WHEN** one agent starts captures on two tabs under the same valid Profile lease
- **THEN** each tab SHALL retain independent filters, sequence state, buffers, and stop behavior
