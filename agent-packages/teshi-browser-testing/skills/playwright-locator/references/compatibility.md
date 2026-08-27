# Teshi browser-agent compatibility and setup

## Supported contract

| Component | Required value |
|---|---|
| Teshi CLI | `>=0.7.10 <0.8.0` |
| Browser-agent schema | `1` |
| Broker/extension protocol | `1` |
| teshi-bridge extension | `>=0.7.10 <0.8.0` |
| Chromium | 116 or newer with Manifest V3 and `chrome.debugger` |
| Operating systems | Windows x86_64, Linux x86_64, macOS arm64 |
| MCP | `2026-07-28`; legacy initialization is accepted for `2025-11-25`, `2025-06-18`, and `2024-11-05` |

The machine-readable declaration is `compatibility.json` at the plugin root. Teshi's broker and MCP server are same-host integrations. They do not grant an agent access to a browser on another host or OS user session.

## Install and connect the extension

1. Find `teshi-bridge` under the Teshi release's `share/teshi-bridge` directory or the browser-testing package's `extension/teshi-bridge` directory.
2. Open `chrome://extensions` in Chrome, Edge, or another compatible Chromium browser.
3. Enable Developer mode, choose **Load unpacked**, and select the `teshi-bridge` directory containing `manifest.json`.
4. Open the extension popup, optionally assign a recognizable profile label, and connect to the active local Teshi project.
5. Confirm the popup reports broker protocol 1 and a ready session. Close DevTools on the target tab if it reports a debugger conflict.

The package supports unpacked/installer installation. Do not assume Chrome Web Store publication.

## Allocate profiles to agents

Create a dedicated Chromium profile for each concurrent agent. Install the extension separately in every profile and assign display-only labels such as `agent-a` and `agent-b`. All profiles connect to the same loopback broker port; their persisted opaque extension instance IDs keep routing separate.

Run `teshi browser sessions`, select by explicit opaque instance ID, then run `teshi browser tabs --session <id>`. Each agent must acquire a lease only for its selected profile. Never use a profile label or numeric tab ID as a global routing key.

## Recover common failures

| Code/health | Recovery |
|---|---|
| `browser_unavailable` | Start the Teshi Chrome sidecar, reload the extension, and click Connect in its popup. |
| `incompatible_browser_session` | Install the extension and CLI ranges listed above; do not attempt a command. |
| `browser_session_disconnected` | Reopen/reload that profile and wait for a fresh heartbeat. |
| `ambiguous_browser_target` | List sessions and tabs, then provide the complete explicit target. |
| `browser_session_busy` | Use another dedicated profile or wait for the current bounded lease to be released. |
| `debugger_conflict` | Close DevTools or another debugger/automation client on the target tab. |
| `expired_browser_lease` | Acquire a new lease; never reuse the old token. |
| `stale_browser_target` | Take a new snapshot and resolve against its new page revision. |
| `browser_operation_timeout` | Check session health once, then retry only after fixing connectivity. |

Session discovery deliberately omits lease secrets. Avoid persisting or logging lease tokens, page content, URLs, titles, and screenshot references beyond what the user's task requires.
