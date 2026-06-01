# Browser modes (teshi-desktop)

teshi-desktop supports two browser backends for BDD locator recording. Both expose the same WebSocket commands (`get_page_snapshot`, `highlight_selector`, `clear_highlight`; Chrome mode also supports `activate_tab`) and write `.teshi/cdp-endpoint.json` for agents.

## How Chrome mode communicates

```text
┌─────────────────┐     POST /heartbeat (~1.5s)          ┌──────────────────────┐
│  teshi-bridge   │ ──────────────────────────────────► │  Python bridge       │
│  Chrome ext     │ ◄────────────────────────────────── │  127.0.0.1:17373     │
│  (active tab)   │     JSON { cmd? }                     │  browser_service.py  │
│                 │     WS /extension/frames (TSH1 JPEG)  │                      │
└─────────────────┘                                       └──────────┬───────────┘
        │ CDP debugger (screencast + locator cmds)                     │ WebSocket
        ▼                                                            ▼
┌─────────────────┐                                       ┌──────────────────────┐
│  Your web app   │                                       │  teshi-desktop       │
│  (e.g. enhook)  │                                       │  preview + agents    │
└─────────────────┘                                       └──────────────────────┘
```

- **Extension control plane**: HTTP `POST /v1/bridge/heartbeat` (~1.5 s) for `project_root`, tabs, and queued `cmd` objects. Small replies go to `POST /v1/bridge/response`.
- **Extension preview data plane**: WebSocket `extension_frame_ws_url` from `GET /v1/bridge` (same host/port as agent `ws_url`, path `/extension/frames`). Binary **TSH1** packets: magic + JSON meta (`tab_id`, `url`, `seq`) + raw JPEG. The bridge base64-encodes once in a thread pool and broadcasts `{"type":"frame","data":"..."}` to teshi-desktop (same agent WebSocket protocol as before).
- **Preview capture**: CDP `Page.startScreencast` at **~10 fps** (JPEG quality **70**, fit inside **1920×1080**, no upscaling). Locator commands pause screencast briefly, then resume.
- **Fallback**: If screencast is unavailable, the extension may POST raw JPEG to `POST /v1/bridge/frame` (no JSON base64). Large JSON frames on `/v1/bridge/response` are deprecated.
- **Agent ↔ bridge**: WebSocket (`ws_url` in `.teshi/cdp-endpoint.json`).
- **Desktop**: starts/stops the Python process; polls discovery; `POST /v1/bridge/activate_tab` to switch Chrome tabs.

Chrome may show **“Chrome is being controlled by automated test software”** while screencast runs (debugger attached). This is expected for CDP-based live preview. Keep the active tab on **http(s)**.

The popup **Connect to teshi** button sends one heartbeat immediately and refreshes status text. The green **OK** badge means the last heartbeat succeeded.

## Connect Chrome (recommended for locators)

Use your **daily Chrome** with real login sessions (SSO, cookies).

1. Install the unpacked extension from `C:\Program Files\teshi\share\teshi-bridge` (or `extension/teshi-bridge` in repo for development; see [extension README](../extension/teshi-bridge/README.md)).
2. Open the app under test in Chrome and select the target **tab**.
3. In teshi-desktop, click **Connect Chrome** in the Browser panel.
4. Wait until the live stream appears (extension connected; check the status dot tooltip if needed).
5. Select a Gherkin step and run the **bdd-locator** skill in the agent terminal.

While waiting for the extension, the panel shows setup steps only (no stream). Once connected, the panel shows:

- A **tab strip** for tabs in the **current Chrome window** (title + favicon). The active tab is highlighted. Click another debuggable tab to activate it in Chrome; the **~10 fps** preview follows the active http(s) tab.
- Read-only active-tab URL, **Refresh** to restart the stream, and zoom controls (in-panel **Go** is not available in chrome mode).
- If the stream stalls, the panel shows an error (check the active tab is http(s), not `chrome://`; reload the extension and **Connect Chrome** again).

`chrome://` and extension pages appear in the strip but are not selectable (not CDP-debuggable).

Discovery: `GET http://127.0.0.1:17373/v1/bridge` returns `tabs`, `active_tab_id`, `page_url`, `extension_connected`, `extension_frame_ws_url`, `last_frame_error`, and `last_frame_age_ms`.  
`cdp-endpoint.json` includes `"mode": "chrome"` and `extension_frame_ws_url` when available.

## Start Embedded (preview / CI alignment)

Launches **headless Playwright Chromium** with a live JPEG stream in the panel (1920×1080 viewport, ~8 fps). Use for local or staging URLs without the extension.

1. Click **Start Embedded**.
2. Navigate with the in-panel address bar.
3. Record locators the same way via **bdd-locator**.

`cdp-endpoint.json` includes `"mode": "embedded"`.

## Mutual exclusion

Only one session runs at a time. Starting Chrome disconnects Embedded and vice versa.

## CI / `teshi run`

The test runner uses its own headless browser configuration. Confirmed locators in `{feature}.locators.md` should use selectors that work in both Chrome recording and CI execution.
