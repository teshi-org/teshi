# Browser Agent Contracts

Teshi's browser bridge exposes one versioned operation model to the CLI, the
local STDIO MCP adapter, and interactive shells. Protocol version `1` adds
explicit browser-session targeting while retaining the legacy single-session
messages described below.

## Legacy compatibility baseline

Existing extensions may send a heartbeat containing `project_root`, `url`,
`title`, `active_tab_id`, and a flat `tabs` array. Existing clients may send a
WebSocket command containing only `cmd`, `request_id`, and command-specific
fields. A bridge with exactly one eligible browser target continues to resolve
these messages implicitly. When more than one target is eligible, it returns
`ambiguous_browser_target` and performs no browser action.

The machine-readable compatibility fixtures live in
`resources/browser_contract_fixtures.json`. They cover the legacy heartbeat,
command, response, preview-frame metadata, and `.teshi/cdp-endpoint.json`
discovery payload.

## Version 1 identity and routing

Every extension installation persists an opaque `extension_instance_id` in
profile-local extension storage. Heartbeats add `protocol_version`,
`extension_version`, `profile_label`, browser metadata, and a `windows` array.
The canonical target is:

```json
{
  "extension_instance_id": "profile-opaque-id",
  "window_id": 1,
  "tab_id": 42
}
```

Commands, responses, diagnostics, and preview frames carry the target and a
unique `request_id`. Browser mutation and locator acquisition also carry an
exclusive instance-level `lease_token`.

## Shared operations

The shared operation names are `list_browser_sessions`, `list_browser_tabs`,
`acquire_browser_lease`, `renew_browser_lease`, `release_browser_lease`,
`get_page_snapshot`, `resolve_playwright_locator`,
`verify_playwright_locator`, and `capture_browser_evidence`. Successful results
include `schema_version`, `operation`, and `request_id`. Failures include a
stable error `code`, an actionable `error`, and non-sensitive recovery metadata
when available.

The broker is loopback-only by default. Session inventory must never include
profile filesystem paths, cookies, storage values, form secrets, or page HTML.

## Filtered network capture

Enhanced network capture requires the advertised
`p1.filtered_network_capture` and `p1.network_batch_transport` features. A
start command supplies a complete leased target, one or more normalized exact
`allowed_hostnames`, a broker-generated opaque `capture_id`, and optional raw
request-body retention with a positive byte limit. Clients fail closed when
either feature is unavailable; they do not fall back to legacy unfiltered
capture.

The extension filters parsed HTTP(S) hostnames before requesting post data from
CDP. Matching requests with `hasPostData` may include a bounded `request_body`
object:

```json
{
  "encoding": "utf8",
  "body": "{\"name\":\"example\"}",
  "captured_size": 18,
  "original_size": 18,
  "truncated": false,
  "unavailable_reason": null
}
```

Request bodies are raw and are not covered by metadata redaction. List results
omit request and response bodies; detail results include a retained request
body, while response-body retrieval remains separately explicit.

Captured events travel on the authenticated extension WebSocket as bounded
`network_batch` messages. Every event carries a monotonic sequence number and
is correlated by extension instance, complete target, and capture ID. The
broker returns `network_ack` with the highest contiguous processed sequence.
The extension retains unacknowledged events in a bounded queue, resends them
after reconnect, and reports cumulative dropped-event diagnostics. The broker
deduplicates retransmission and rejects mismatched or oversized batches.
