# Browser modes (teshi-desktop)

teshi-desktop supports two browser backends for BDD locator recording. Both expose the same WebSocket commands (`get_page_snapshot`, `highlight_selector`, `clear_highlight`) and write `.teshi/cdp-endpoint.json` for agents.

## How Chrome mode communicates

```text
┌─────────────────┐     POST /heartbeat (every 1s)      ┌──────────────────────┐
│  teshi-bridge   │ ──────────────────────────────────► │  Python bridge       │
│  Chrome ext     │ ◄────────────────────────────────── │  127.0.0.1:17373     │
│  (active tab)   │     JSON { cmd? } + POST /response    │  browser_service.py  │
└─────────────────┘                                       └──────────┬───────────┘
        │ CDP debugger attach                                          │ WebSocket
        ▼                                                            ▼
┌─────────────────┐                                       ┌──────────────────────┐
│  Your web app   │                                       │  Cursor Agent / CLI    │
│  (e.g. enhook)  │                                       │  get_page_snapshot     │
└─────────────────┘                                       └──────────────────────┘
        ▲                                                            │
        │                                                            │
┌─────────────────┐     Connect Chrome starts bridge                 │
│  teshi-desktop  │ ────────────────────────────────────────────────┘
└─────────────────┘     writes .teshi/cdp-endpoint.json
```

- **Extension ↔ bridge**: HTTP only (`/v1/bridge/heartbeat`, `/v1/bridge/response`). No WebSocket from the extension (MV3-safe). CDP screenshots are captured in the extension and forwarded as frame payloads over `/v1/bridge/response`.
- **Agent ↔ bridge**: WebSocket (`ws_url` in `.teshi/cdp-endpoint.json`).
- **Extension ↔ page**: Chrome `debugger` API (CDP) on the **active tab** only.
- **Desktop**: starts/stops the Python process; does not talk to the extension directly.

The popup **Connect to teshi** button sends one heartbeat immediately and refreshes status text. The green **OK** badge means the last heartbeat succeeded.

## Connect Chrome (recommended for locators)

Use your **daily Chrome** with real login sessions (SSO, cookies).

1. Install the unpacked extension from `C:\Program Files\teshi\share\teshi-bridge` (or `extension/teshi-bridge` in repo for development; see [extension README](../extension/teshi-bridge/README.md)).
2. Open the app under test in Chrome and select the target **tab**.
3. In teshi-desktop, click **Connect Chrome** in the Browser panel.
4. Wait until the panel shows **extension: connected**.
5. Select a Gherkin step and run the **bdd-locator** skill in the agent terminal.

Discovery: `http://127.0.0.1:17373/v1/bridge`  
`cdp-endpoint.json` includes `"mode": "chrome"`.

## Start Embedded (preview / CI alignment)

Launches **headless Playwright Chromium** with a live JPEG stream in the panel (1920×1080). Use for local or staging URLs without the extension.

1. Click **Start Embedded**.
2. Navigate with the in-panel address bar.
3. Record locators the same way via **bdd-locator**.

`cdp-endpoint.json` includes `"mode": "embedded"`.

## Mutual exclusion

Only one session runs at a time. Starting Chrome disconnects Embedded and vice versa.

## CI / `teshi run`

The test runner uses its own headless browser configuration. Confirmed locators in `{feature}.locators.md` should use selectors that work in both Chrome recording and CI execution.
