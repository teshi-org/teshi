## ADDED Requirements

### Requirement: Feature-negotiated network stream
The extension and broker SHALL negotiate support for filtered network capture and acknowledged WebSocket batches before enabling enhanced capture, and SHALL fail closed when either capability is unavailable.

#### Scenario: Older extension is connected
- **WHEN** a new CLI requests enhanced capture from an extension that does not advertise the required capability
- **THEN** Teshi SHALL reject the request and SHALL NOT fall back to unfiltered legacy capture

### Requirement: Network-aware WebSocket lifecycle
The extension SHALL keep its authenticated broker WebSocket connected or reconnecting while any network capture is active or any network batch remains unacknowledged, regardless of screencast state.

#### Scenario: Preview is disabled
- **WHEN** network capture is active without a browser preview
- **THEN** the extension SHALL maintain the WebSocket needed to deliver and acknowledge network batches

### Requirement: Targeted debugger lifecycle reporting
The extension SHALL manage debugger attachment independently per tab and SHALL report an abnormal detach for the affected target without silently changing another target's capture state.

#### Scenario: One captured tab closes
- **WHEN** a Profile captures two tabs and one tab closes
- **THEN** the extension SHALL end and report only the closed tab's capture
