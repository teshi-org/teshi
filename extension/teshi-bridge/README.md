# teshi-bridge (Chrome extension)

Connects your **Chrome window tabs** to [teshi-desktop](../../desktop/README.md) for BDD locator recording with real login sessions. One active tab is mirrored at a time; teshi-desktop can list tabs and switch which tab is active.

## Icons

Toolbar and store icons are PNGs under `icons/`, generated from the same asset as teshi-desktop (`desktop/src-tauri/icons/icon.ico`). After updating the desktop icon, regenerate sizes (16 / 32 / 48 / 128) and reload the extension in Chrome.

## Install (unpacked)

1. Open Chrome and go to `chrome://extensions`.
2. Enable **Developer mode** (top right).
3. Click **Load unpacked** and select:
   - Installed MSI path (recommended): `C:\Program Files\teshi\share\teshi-bridge`
   - Repo path (development): `extension/teshi-bridge`
4. Pin the extension if you want to see when it is active (optional).

## Use with teshi-desktop

1. In **Google Chrome** (or another Chromium browser), open the tabs you need in one window on an **http(s)** page.
2. In teshi-desktop, open your project and click **Connect Chrome** in the Browser panel.
3. The extension sends **metadata heartbeats** (~1.5 s) to `127.0.0.1:17373` and a **~10 fps CDP screencast** (JPEG quality **70**, up to **1920×1080**, no upscaling) over **WebSocket** (`extension_frame_ws_url` from `GET /v1/bridge`). The Python bridge broadcasts frames to teshi-desktop over the agent WebSocket.
4. When connected, use the **tab strip** in the Browser panel to switch tabs, or activate tabs in Chrome directly. Optional: click the extension icon → **Connect to teshi** to wake the service worker and restart the stream.
5. Select a Gherkin step and run the **bdd-locator** agent skill in the terminal.

After changing extension files, click **Reload** on `chrome://extensions` for teshi-bridge, then **Disconnect / Connect Chrome** in teshi-desktop so `browser_service.py` restarts if needed.

If the preview stalls:

- Confirm the extension is enabled and the active tab is `http://` or `https://` (not `chrome://`).
- Click **Refresh** in the Browser panel or reload the extension.
- Close DevTools on the target tab when using locator CDP commands (debugger attach conflicts).
- Chrome may show the automation banner while screencast is active; this is normal for CDP preview.

## Permissions

- **debugger** — CDP screencast preview and locator snapshot/highlight (preview pauses briefly during locator commands).
- **tabs** — list tabs in the current window and activate a tab when teshi-desktop requests it.
- **activeTab** — limit scope to user-visible browsing.
- **alarms** — periodic wake for MV3 service worker.
- **127.0.0.1 / ws://127.0.0.1** — local bridge discovery, heartbeat, extension frame WebSocket, and HTTP fallback upload.
- **http(s)://\*** — pages that can be debugged and screencast.

## Protocol (extension ↔ bridge)

**Metadata** — `POST /v1/bridge/heartbeat` every ~1.5 s with `project_root`, `url`, `title`, `active_tab_id`, and `tabs[]` (`id`, `title`, `url`, `active`, `favIconUrl`, `debuggable`). The response may include a `cmd` object and `stream_restart`.

**Preview (primary)** — CDP `Page.startScreencast` → binary **TSH1** WebSocket uplink to `extension_frame_ws_url`:

- `[4B magic 'TSH1'][4B meta_len LE][meta JSON][JPEG bytes]`
- `stream_hello` JSON with `project_root` on connect; bridge replies `stream_hello_ack`.

**Preview (fallback)** — `POST /v1/bridge/frame` with `Content-Type: image/jpeg` and query `project_root`, `tab_id`, `url` if screencast cannot start.

**Commands** (on heartbeat response `cmd`) — `get_page_snapshot`, `highlight_selector`, `clear_highlight`, `activate_tab` (with `tab_id`). Replies are posted to `/v1/bridge/response` (JSON only; no large frames). Locator commands pause screencast until they finish.

**Desktop discovery** — `GET /v1/bridge` includes `extension_frame_ws_url`, `last_frame_error`, and `last_frame_age_ms` for stream health.
