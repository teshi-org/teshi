# hosted-ui-compatibility Specification

## Purpose
TBD - created by archiving change decouple-nightly-cli-and-hosted-web-ui. Update Purpose after archive.
## Requirements
### Requirement: Hosted UI publishes a compatibility manifest
The hosted application SHALL publish `/app/ui-manifest.json` with a versioned schema containing immutable UI source/deployment identity, control and preview protocol versions, a complete minimum compatible CLI build identity including minimum monotonic `build_sequence`, supported release channel, and an actionable nightly upgrade target.

#### Scenario: Pages artifact is validated
- **WHEN** the Pages workflow validates the generated site
- **THEN** the manifest SHALL be parseable, complete, internally consistent with compatibility constants compiled into the WASM bundle, and tied to the recorded Teshi source SHA

### Requirement: Compatibility completes before business connection
The hosted UI and daemon SHALL exchange and validate their protocol versions and build identities during the hello handshake, before accepting requests, events, subscriptions, or preview attachment.

#### Scenario: CLI satisfies the hosted minimum
- **WHEN** the daemon presents a valid released identity whose channel is supported, whose build sequence meets or exceeds the hosted minimum, and whose required protocols overlap
- **THEN** the UI SHALL complete the business session using the selected protocol versions

#### Scenario: Identity is incomplete or unverifiable
- **WHEN** a production hosted session receives a development/incomplete identity, including build sequence `0`, without an explicit development override
- **THEN** the UI SHALL treat the CLI as incompatible and SHALL NOT establish the business session

### Requirement: Incompatible CLI fails closed with upgrade guidance
If the local CLI does not satisfy the latest UI minimum or required protocol versions, the UI SHALL refuse business connection and SHALL display a clear instruction to upgrade the nightly CLI.

#### Scenario: Nightly CLI is older than the minimum
- **WHEN** the daemon build sequence is lower than `minimum_cli.build_sequence`
- **THEN** the UI SHALL issue no business request and SHALL show the current CLI identity, required minimum, and nightly upgrade action

#### Scenario: Protocol versions do not overlap
- **WHEN** no compatible control or preview protocol can be selected
- **THEN** the UI SHALL close or retain only the failed handshake state and SHALL show the incompatibility reason

### Requirement: Compatibility never falls back to historical UI
An incompatibility SHALL NOT cause the hosted application to download, redirect to, or execute a historical Web UI artifact.

#### Scenario: Old CLI opens the latest hosted UI
- **WHEN** compatibility negotiation rejects the local CLI
- **THEN** the canonical `/app/` page SHALL remain on the explicit upgrade state
- **AND** it SHALL NOT select an older asset set
