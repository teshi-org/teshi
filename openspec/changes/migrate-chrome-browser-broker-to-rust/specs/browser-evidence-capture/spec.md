## ADDED Requirements

### Requirement: Target-isolated screenshot and preview evidence
The broker SHALL preserve existing screenshot and preview metadata, size limits, and target identity. Binary preview frames SHALL be delivered only to subscribers authorized for the same complete extension Profile/window/tab target.

#### Scenario: Two Profiles publish frames
- **WHEN** two Profiles publish screencast frames with matching numeric window or tab IDs
- **THEN** each subscriber SHALL receive frames only for its selected extension instance and target

#### Scenario: Preview frame exceeds the limit
- **WHEN** a frame exceeds the configured dimensions, pixel count, or byte bound
- **THEN** the broker SHALL reject it with a diagnostic and SHALL preserve the last valid frame without unbounded allocation

### Requirement: Bounded redacted Console capture
Console events SHALL remain target-scoped, filtered by configured levels and age, bounded by event count and bytes, and redacted before persistence or diagnostics.

#### Scenario: Sensitive console data is captured
- **WHEN** a captured Console event contains configured sensitive fields
- **THEN** stored and listed diagnostics SHALL redact those values and SHALL report truncation where a bound is reached

#### Scenario: A target disconnects
- **WHEN** a Profile or target disconnects during Console capture
- **THEN** only that target's capture SHALL terminate with an explicit reason and unrelated captures SHALL continue

### Requirement: Acknowledged and deduplicated Network capture
Network capture SHALL require a lease and an explicit normalized hostname allowlist. Request/response records SHALL be correlated by target and capture ID, bounded by age/count/bytes, and delivered with monotonic per-capture sequence numbers and contiguous acknowledgements. Listing SHALL omit bodies; raw body retrieval SHALL require explicit authorization.

#### Scenario: A matching request is retransmitted
- **WHEN** an extension resends an already accepted event after reconnect
- **THEN** the broker SHALL acknowledge the accepted contiguous sequence without counting or storing the event twice

#### Scenario: A sequence gap is received
- **WHEN** a batch contains a missing or out-of-order sequence
- **THEN** the broker SHALL not advance the contiguous acknowledgement past the gap and SHALL expose a loss diagnostic

#### Scenario: A non-allowlisted host is observed
- **WHEN** the extension observes a request whose parsed HTTP(S) hostname is not on the capture allowlist
- **THEN** the event and its request/response body SHALL not be retained by the broker

#### Scenario: Network body is listed or fetched
- **WHEN** a caller lists Network requests or asks for a retained body
- **THEN** list results SHALL contain metadata only and body details SHALL require the matching target, capture, lease, project authorization, and configured byte limit

### Requirement: Safe managed browser artifacts
Screenshot, PDF, upload, and cleanup operations SHALL validate the project root and payload bounds before file access, keep paths within the project's managed artifact directory, and avoid logging raw page or body data by default.

#### Scenario: Artifact path attempts traversal
- **WHEN** an operation supplies a path that traverses outside the managed browser artifact directory
- **THEN** the broker SHALL reject the operation before reading, writing, or deleting any file

#### Scenario: Artifact payload exceeds its bound
- **WHEN** an artifact payload exceeds the configured byte, dimension, or pixel bound
- **THEN** the broker SHALL reject it before persistence and SHALL leave existing user files unchanged
