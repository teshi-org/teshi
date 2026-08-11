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
