# Browser modes (teshi-desktop)

teshi-desktop supports two browser backends for BDD locator recording. Both expose the same WebSocket commands (`navigate`, `get_page_snapshot`, `highlight_selector`, `clear_highlight`, `execute_locator`; Chrome mode also supports `activate_tab`) and write `.teshi/cdp-endpoint.json` for agents.

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
- **Preview capture**: CDP `Page.startScreencast` only (JPEG quality **70**, fit inside **1920×1080**, no upscaling). Frames are sent when the page repaints (scroll, animation, etc.); a static page keeps the last frame and the panel may show **Preview idle**. Locator commands pause screencast briefly, then resume. If screencast cannot start, the extension reports `frame_error` (no HTTP capture fallback). Large JSON frames on `/v1/bridge/response` are deprecated.
- **Agent ↔ bridge**: WebSocket (`ws_url` in `.teshi/cdp-endpoint.json`).
- **Desktop**: starts/stops the Python process; polls discovery; `POST /v1/bridge/activate_tab` to switch Chrome tabs.

Chrome may show **“Chrome is being controlled by automated test software”** while screencast runs (debugger attached). This is expected for CDP-based live preview. Keep the active tab on **http(s)**.

The popup **Connect to teshi** button sends one heartbeat immediately and refreshes status text. The green **OK** badge means the last heartbeat succeeded.

## Connect Chrome (default for locators)

Use a **dedicated recording Chrome profile** with real login sessions (SSO, cookies). Install `teshi-bridge` only in that profile so daily browsing and other browser automation tools do not attach to the same debugger target.

1. Start the dedicated recording Chrome profile manually.
2. Install the unpacked extension from `C:\Program Files\teshi\share\teshi-bridge` (or `extension/teshi-bridge` in repo for development; see [extension README](../extension/teshi-bridge/README.md)).
3. Open the app under test in Chrome and select the target **tab**.
4. In teshi-desktop, click **Connect Chrome** in the Browser panel.
5. Wait until the live stream appears (extension connected; check the status dot tooltip if needed).
6. Select a Gherkin step and run the **bdd-locator** skill in the agent terminal.

While waiting for the extension, the panel shows setup steps only (no stream). Once connected, the panel shows:

- A **tab strip** for tabs in the **current Chrome window** (title + favicon). The active tab is highlighted. Click another debuggable tab to activate it in Chrome; the screencast preview updates on the active http(s) tab when the page repaints.
- Read-only active-tab URL, **Refresh** to restart the stream, and zoom controls (in-panel **Go** is not available in chrome mode).
- If preview is **idle**, interact in Chrome to refresh. If the stream is **disconnected** or shows `frame_error`, check the active tab is http(s), reload the extension, and **Connect Chrome** again.

`chrome://` and extension pages appear in the strip but are not selectable (not CDP-debuggable).

Chrome mode allows agent-driven navigation only for explicit URL steps, for example a Background step that says to open `https://example.com`. Skills should call `teshi browser navigate <url>` only when the URL is present in the step text; they should not invent hidden navigation. Other page changes should happen through confirmed step bindings or direct user action.

Discovery: `GET http://127.0.0.1:17373/v1/bridge` returns `tabs`, `active_tab_id`, `page_url`, `extension_connected`, `extension_frame_ws_url`, `last_frame_error`, and `last_frame_age_ms`.  
`cdp-endpoint.json` includes `"mode": "chrome"` and `extension_frame_ws_url` when available.

## Step bindings

Confirmed locators are stored in `.teshi/step-bindings/{feature}.json` and should be committed with the project. The older `{feature}.locators.md` files are deprecated and are no longer written or read by the recording/replay workflow.

`navigate` bindings store the URL and are replayed as first-class setup actions. `execute_locator` supports `click`, `fill`, `assert_visible`, `assert_text`, `select`, and `press_key`. Unknown actions fail with `unsupported_action`; missing values for value-based actions fail with `missing_value`.

Values are stored in git as part of the binding. Use placeholders such as `${LOGIN_PW}` for secrets and never commit real passwords, tokens, or private customer data.

## Start Embedded (preview / CI alignment)

Launches **headless Playwright Chromium** with a live JPEG stream in the panel (1920×1080 viewport, ~8 fps). Use for local or staging URLs without the extension.

1. Click **Start Embedded**.
2. Navigate with the in-panel address bar.
3. Record locators the same way via **bdd-locator**.

`cdp-endpoint.json` includes `"mode": "embedded"`.

## Mutual exclusion

Only one session runs at a time. Starting Chrome disconnects Embedded and vice versa.

## CI / replay

Use `teshi browser replay --non-interactive` for CI-style browser setup from confirmed step-bindings. Interactive replay remains the default for terminal agents so each step can be reviewed before execution.

## Diagnostics

Set `TESHI_BROWSER_DEBUG=1` before starting teshi and running agent commands to persist JSONL diagnostics:

- `.teshi/logs/browser-bridge.log` for Python sidecar and Chrome extension command forwarding.
- `.teshi/logs/cli-browser.log` for `teshi browser` and `teshi steps` command timing, request IDs, and errors.

For snapshot timeouts, reload `teshi-bridge` in `chrome://extensions`, click **Connect Chrome** again, confirm `.teshi/cdp-endpoint.json` has `extension_connected: true`, then rerun `TESHI_BROWSER_DEBUG=1 teshi browser snapshot` and inspect the logs above. Use `teshi browser snapshot --timeout-ms 90000` on heavy pages.

Terminal agents verify locators with `teshi browser execute --selector <css> --action <action>` and `--value-arg <text>` for `fill`, `assert_text`, `select`, and `press_key` (same flags as `teshi steps propose`).
