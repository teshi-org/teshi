## 1. Protocol and CLI Contract

- [x] 1.1 Add negotiated filtered-network and WebSocket-batch capability identifiers to shared discovery and compatibility fixtures
- [x] 1.2 Require repeatable exact hostnames and add request-body limits in Rust browser operation types and CLI parsing
- [x] 1.3 Preserve metadata-only list output while adding bounded raw request-body fields to detail responses

## 2. Broker Capture State

- [x] 2.1 Extend target-scoped capture state with capture ID, normalized hostnames, request-body limits, sequence watermarks, and delivery diagnostics
- [x] 2.2 Validate and merge authenticated network batches with target/capture isolation, hostname defense, deduplication, and contiguous acknowledgements
- [x] 2.3 Add clear and stop sequence barriers plus lifecycle termination reasons

## 3. Extension Capture Runtime

- [x] 3.1 Replace singleton debugger ownership with per-tab role-aware sessions for preview, commands, console, and network capture
- [x] 3.2 Filter matching HTTP(S) hostnames before retaining events and capture bounded request bodies through CDP
- [x] 3.3 Keep captures active across active-tab changes and report tab closure or external debugger detach

## 4. WebSocket Delivery

- [x] 4.1 Send bounded capture batches with capture ID, monotonic sequence, retry queue, and drop counters over the authenticated extension WebSocket
- [x] 4.2 Accept, deduplicate, and acknowledge network batches in the Python WebSocket service
- [x] 4.3 Keep the extension socket reconnecting while capture or unacknowledged data exists and retain legacy HTTP parsing only for compatibility

## 5. Documentation and Validation

- [x] 5.1 Update CLI, browser mode, extension, protocol, and package compatibility documentation
- [x] 5.2 Add Rust, Python, and behavior-level extension tests for filters, bodies, batching, reconnect, lifecycle, and concurrent Profile/Tab isolation
- [x] 5.3 Run OpenSpec validation, extension/Python/package suites, formatting, check, test, clippy, and documentation gates
