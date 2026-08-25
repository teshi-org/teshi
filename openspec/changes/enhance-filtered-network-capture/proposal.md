## Why

Teshi's existing browser network capture records all request metadata for a selected tab, omits request bodies, and posts every event separately over HTTP. Multi-agent testing needs explicit hostname-scoped capture with bounded request bodies and reliable, isolated delivery across several browser profiles.

## What Changes

- Require one or more exact hostnames when starting network capture for a leased Profile/window/tab target.
- Capture bounded request bodies for matching HTTP(S) requests, including POST, PUT, PATCH, and any request reported by CDP as carrying post data.
- Keep request bodies raw and unredacted while preserving explicit byte limits, encoding metadata, and truncation or unavailability markers.
- Replace per-event HTTP uploads with authenticated WebSocket batches using capture identifiers, monotonic sequence numbers, acknowledgements, bounded retries, deduplication, and drop counters.
- Refactor extension debugger ownership so explicit network capture survives active-tab changes and can run concurrently on multiple tabs and Profiles.
- Preserve metadata-only list output, explicit response-body retrieval, existing header/query redaction, and local-only broker access.
- Defer Desktop UI, HAR export, performance timing, initiator/redirect enrichment, and WebSocket/SSE payload capture.

## Capabilities

### New Capabilities

- `filtered-browser-network-capture`: Hostname-filtered, request-body-aware, explicitly controlled network capture with bounded WebSocket delivery.

### Modified Capabilities

- `multi-browser-session-broker`: Extend parallel Profile isolation to independent network capture streams and capture ownership.
- `browser-extension-connection`: Keep authenticated extension WebSocket connections alive for network capture and report abnormal debugger detach lifecycle events.
- `external-agent-testing-interface`: Extend machine-readable CLI operations with mandatory hostname filters, request-body options, and capture delivery diagnostics.

## Impact

- Affects the Rust browser operation model and CLI in `crates/teshi-engine` and `crates/teshi-tui`.
- Affects broker state and extension WebSocket routing in `resources/browser_agent_broker.py` and `resources/browser_service.py`.
- Affects CDP attachment, network event capture, and transport in `extension/teshi-bridge/background.js`.
- Updates browser protocol fixtures, compatibility declarations, agent package metadata, tests, and browser documentation.
- Introduces no new Chromium extension permissions or third-party dependencies.
