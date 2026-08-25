## Context

The current extension uses one `attachedTabId` for preview, locator, console, and network work. Changing the active tab detaches that debugger and silently ends capture. Network events are posted individually to `/v1/bridge/response`, request bodies are omitted, and capture has no hostname filter. The broker already identifies Profiles by persistent `extension_instance_id`, isolates state by Profile/window/tab, and exposes exclusive Profile leases.

## Goals / Non-Goals

**Goals:**

- Keep captures independent across Profiles and tabs while sharing one per-user broker.
- Require explicit exact-hostname filters and explicit start/stop commands.
- Capture bounded raw request bodies for matching HTTP(S) requests.
- Deliver network events over an authenticated, reconnecting WebSocket with bounded batching, acknowledgement, deduplication, and drop diagnostics.
- Preserve existing CLI list/detail semantics and response-body opt-in behavior.

**Non-Goals:**

- Desktop network UI, HAR export, permanent persistence, timing/initiator/redirect enrichment, or WebSocket/SSE payload capture.
- Concurrent leases from different agents against the same Profile.
- Removal of existing header and query-field redaction.

## Decisions

### Use the existing authenticated extension WebSocket

The `/extension/frames` socket already authenticates a Profile with `stream_hello`, carries direct commands and responses, and accepts binary screencast frames. It will additionally carry `network_batch` and `network_ack` JSON messages. This avoids a second connection and the overhead of one HTTP request per CDP event. The connection lifetime will depend on any active screencast, network capture, direct command, or unacknowledged batch.

Each capture uses a broker-generated opaque `capture_id`. Extension events receive monotonically increasing sequence numbers and are retained in a bounded queue until a contiguous acknowledgement arrives. Reconnect resends unacknowledged events. The broker deduplicates by Profile, complete target, capture ID, and sequence number. Queue overflow is reported as cumulative drop counters rather than allowing unbounded memory growth.

### Filter before body retrieval and transport

`network start` requires at least one repeatable exact hostname. The CLI and extension normalize hostnames to lowercase ASCII without schemes, credentials, ports, paths, wildcards, or trailing dots. The extension checks the parsed HTTP(S) request URL before reading a body or enqueuing an event; the broker repeats the check defensively.

For matching requests with `hasPostData`, the extension uses inline CDP `postData` when available and otherwise calls `Network.getRequestPostData` immediately. Bodies are byte-bounded and represented as UTF-8 or Base64 with captured size, known original size where available, and truncation/unavailability metadata. Bodies remain raw. List output excludes bodies; detail output includes the retained request body.

### Manage debugger ownership per tab and role

The extension will replace the single debugger owner with per-tab sessions. Preview, network, console, and short-lived command roles acquire and release the same attachment independently. Active-tab changes move only the preview role. Network capture remains until explicit stop, tab closure, Profile disconnect, broker epoch change, or an external debugger detach. Abnormal detach is reported to the broker with a reason.

Multiple tabs in one Profile may be captured under the same exclusive Profile lease. Different Profiles use separate extension service workers and authenticated sockets, so different agents can capture them concurrently.

### Negotiate the enhanced contract

Discovery will advertise a filtered-network-capture feature and a network-batch transport capability. The enhanced CLI refuses to start capture unless both are available. The existing HTTP `network_event` endpoint remains readable for compatibility, but it cannot populate a new capture. This prevents an older broker or extension from silently ignoring hostname filters and capturing every request.

## Risks / Trade-offs

- [MV3 service worker suspension loses volatile capture state] → Keep an active WebSocket while capturing, bound queues, report disconnect, and require explicit restart after broker or worker epoch loss.
- [WebSocket send does not guarantee broker processing] → Retain events until contiguous ACK and deduplicate retransmission.
- [Request bodies can be large or binary] → Apply configurable byte limits, encode binary data, and expose truncation metadata.
- [CDP cannot always report original request-body size] → Mark size as unknown instead of inventing a value.
- [DevTools or another debugger can detach the extension] → End only the affected target capture and expose the abnormal stop reason.
- [Shared broker event-loop load grows with several Profiles] → Batch events, bound batch bytes/count, and offload no body transformations to unbounded work.

## Migration Plan

1. Add additive Rust/Python/extension fields and advertised feature identifiers.
2. Add WebSocket batch handling while retaining legacy HTTP event parsing.
3. Switch the enhanced CLI start command to require feature negotiation and hostnames.
4. Release CLI, broker, and extension together under the repository's synchronized version.
5. Roll back by disabling the advertised enhanced feature; new clients fail closed instead of using unfiltered legacy capture.

## Open Questions

None. Request bodies are raw by explicit product decision; existing metadata redaction remains unchanged.
