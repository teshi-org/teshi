# teshi-bridge (Chrome extension)

Connects a Chromium browser profile and its tabs to Teshi for BDD recording and verified Playwright locator acquisition with real login sessions. Every installed profile persists a random opaque instance ID; an optional popup label helps humans distinguish profiles, but routing always uses the opaque ID plus window/tab IDs.

## Icons

Toolbar and store icons are PNGs under `icons/`. After updating brand icons, regenerate sizes (16 / 32 / 48 / 128) and reload the extension in Chrome.

## Install (unpacked)

1. Open Chrome and go to `chrome://extensions`.
2. Enable **Developer mode** (top right).
3. Click **Load unpacked** and select:
   - Installed MSI path (recommended): `C:\Program Files\teshi\share\teshi-bridge`
   - Browser-testing package: `teshi-browser-testing/extension/teshi-bridge`
   - Repo path (development): `extension/teshi-bridge`
4. Pin the extension if you want to see when it is active (optional).

## Use with teshi-desktop

1. In **Google Chrome** (or another Chromium browser), open the tabs you need on an **http(s)** page. Use one dedicated browser profile per concurrent agent.
2. In teshi-desktop, open your project and click **Connect Chrome** in the Browser panel.
3. Open the extension popup, set an optional profile label such as `agent-a`, then click **Connect to teshi**. The popup reports the persisted instance ID, extension/protocol versions, and actionable disconnected/incompatible/debugger/stale status.
4. `teshi browser sessions` starts or reuses the per-user broker on `127.0.0.1:17373`; Desktop attaches to that same process. The extension sends metadata heartbeats and opens a session-authenticated WebSocket for screencast frames plus low-latency correlated commands. Heartbeat delivery is the bounded fallback.
5. When connected, explicitly select the profile and tab in the Browser panel. The panel never projects tabs or frames from several profiles into one implicit selection.
6. Select a Gherkin step and run the **bdd-locator** agent skill in the terminal, or use the packaged **playwright-locator** Skill for observational locator acquisition.

After changing extension files, click **Reload** on `chrome://extensions` for teshi-bridge, then **Disconnect / Connect Chrome** in teshi-desktop so `browser_service.py` restarts if needed.

If the preview is idle or stalled:

- A static page shows **Preview idle** in teshi until you scroll or interact in Chrome.
- Confirm the extension is enabled and the active tab is `http://` or `https://` (not `chrome://`).
- Click **Refresh** in the Browser panel or reload the extension.
- Close DevTools on the target tab when using locator CDP commands (debugger attach conflicts).
- Chrome may show the automation banner while screencast is active; this is normal for CDP preview.

## Permissions

- **debugger** — CDP screencast preview and locator snapshot/highlight (preview pauses briefly during locator commands).
- **tabs** — list tabs in the current window and activate a tab when teshi-desktop requests it.
- **activeTab** — limit scope to user-visible browsing.
- **alarms** — periodic wake for MV3 service worker.
- **storage** — persist the opaque extension instance ID and optional display label in this browser profile.

The popup offers three optional, separately approved permissions. `cookies` enables selected-tab Cookie metadata, `contentSettings` enables an allowlisted selected-origin setting read/write surface, and `management` enables extension metadata reads. Teshi never requests these silently. Removing a permission immediately makes the corresponding capability unavailable; safe P0/P1 operations continue working.

Browser permission alone is not authorization. Every P2 call also requires the selected Profile lease and a short-lived, project/profile/caller-bound Teshi grant. Cookie values require a second `cookie-values` grant. Extension enable/disable/uninstall mutations are not implemented.
- **127.0.0.1 / ws://127.0.0.1** — local bridge discovery, heartbeat, and extension frame WebSocket.
- **http(s)://\*** — pages that can be debugged and screencast.

## Protocol (extension ↔ bridge)

Protocol version 1 is the current contract. Legacy single-profile heartbeats remain accepted only through the bounded compatibility adapter.

**Metadata** — `POST /v1/bridge/heartbeat` with `extension_instance_id`, optional `profile_label`, extension/protocol versions, browser metadata, all windows/tabs, active target, and project metadata. Every mutating POST sends `X-Teshi-Broker-Token`, derived from the random token in the discovered WebSocket URL. The response includes the same instance ID, compatibility preflight, only that instance's next `cmd`, and optional `stream_restart`.

**Preview (primary)** — CDP `Page.startScreencast` → binary **TSH1** WebSocket uplink to `extension_frame_ws_url`:

- `[4B magic 'TSH1'][4B meta_len LE][meta JSON][JPEG bytes]`
- `stream_hello` JSON with `project_root`, `extension_instance_id`, and protocol version; the bridge authenticates the session before accepting binary frames.

**Commands** carry a unique request ID and composite instance/window/tab target. P0 supports revision-bound snapshot refs, structured-candidate re-verification, DOM and CDP-pointer activation, typed input/waits, and lease-scoped tab/window/group lifecycle. Direct WebSocket dispatch requires the same broker-start token and atomically claims a command from the heartbeat queue; send failure restores the same request once. Replies echo request, instance, and target and mismatches are quarantined.

**Discovery** — `GET /v1/bridge` includes `sessions[]` with identities, health, browser versions, windows/tabs, public lease state, `extension_frame_ws_url`, and stream diagnostics. Legacy flat fields are populated only when exactly one eligible target exists.
