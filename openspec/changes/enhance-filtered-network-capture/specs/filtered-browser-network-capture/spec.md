## ADDED Requirements

### Requirement: Explicit hostname-scoped capture
Teshi SHALL require at least one exact hostname and a complete leased browser target before starting network capture, and SHALL capture only HTTP(S) requests whose parsed hostname matches the normalized allowlist.

#### Scenario: Agent starts a filtered capture
- **WHEN** an agent starts capture for a leased Profile/window/tab with `api.example.com`
- **THEN** Teshi SHALL capture requests to that exact hostname and SHALL ignore requests to unrelated or suffix-similar hostnames

#### Scenario: Agent omits the hostname
- **WHEN** an agent attempts to start enhanced network capture without a hostname
- **THEN** Teshi SHALL reject the command before enabling CDP network capture

### Requirement: Bounded raw request bodies
Teshi SHALL capture an available request body for each matching request reported by CDP as carrying post data, SHALL retain the raw content without redaction, and SHALL bound it by configured bytes with encoding and truncation metadata.

#### Scenario: Matching POST carries JSON
- **WHEN** a matching POST request carries a JSON body within the configured limit
- **THEN** request detail SHALL return the exact body as UTF-8 with its captured size and no truncation marker

#### Scenario: Matching body exceeds the limit
- **WHEN** a matching request body exceeds the configured byte limit
- **THEN** Teshi SHALL return only the bounded prefix and SHALL report its encoding, captured size, and truncation or unknown-original-size state

### Requirement: Explicit capture lifecycle
Teshi SHALL keep a capture active until explicit stop or a reported target, Profile, broker, or debugger lifecycle failure, and SHALL NOT stop it merely because the browser's active tab changes.

#### Scenario: User activates another tab
- **WHEN** a capture is active on Tab A and the user activates Tab B
- **THEN** capture on Tab A SHALL remain active and continue accepting matching events

#### Scenario: Debugger is externally detached
- **WHEN** DevTools or another debugger detaches the extension from a captured tab
- **THEN** Teshi SHALL stop only that target's capture and SHALL expose an abnormal termination reason

### Requirement: Acknowledged bounded event delivery
The extension SHALL send captured network events as bounded authenticated WebSocket batches with capture identifiers and monotonic sequence numbers, and SHALL retain or explicitly account for events until the broker acknowledges a contiguous sequence.

#### Scenario: Acknowledgement is lost
- **WHEN** a WebSocket batch reaches the broker but its acknowledgement is lost
- **THEN** the extension SHALL resend the unacknowledged batch and the broker SHALL deduplicate it

#### Scenario: Local queue overflows
- **WHEN** disconnected capture traffic exceeds the extension queue limits
- **THEN** the extension SHALL remain bounded and SHALL report cumulative dropped-event diagnostics after reconnection

### Requirement: Summary and detail separation
Network list operations SHALL omit request and response bodies, while request detail SHALL include the retained request body and response body SHALL remain separately opt-in.

#### Scenario: Agent lists captured requests
- **WHEN** an agent lists an active capture
- **THEN** Teshi SHALL return bounded request summaries without request or response body content
