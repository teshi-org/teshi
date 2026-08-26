# Browser modes (teshi-desktop)

teshi-desktop supports two browser backends for BDD locator recording. Both write `.teshi/cdp-endpoint.json`; Chrome mode additionally exposes a versioned multi-profile broker and typed operations for external agents.

## How Chrome mode communicates

```text
┌─────────────────┐     POST /heartbeat (~1.5s)          ┌──────────────────────┐
│  teshi-bridge   │ ──────────────────────────────────► │  Python bridge       │
│  Chrome ext     │ ◄────────────────────────────────── │  127.0.0.1:17373     │
│  (active tab)   │     JSON { cmd? }                     │  browser_service.py  │
│                 │     WS frames + commands + net batches│                      │
└─────────────────┘                                       └──────────┬───────────┘
        │ CDP debugger (screencast + locator cmds)                     │ WebSocket
        ▼                                                            ▼
┌─────────────────┐                                       ┌──────────────────────┐
│  Your web app   │                                       │  teshi-desktop       │
│  target web app │                                       │  preview + agents    │
└─────────────────┘                                       └──────────────────────┘
```

- **Extension control plane**: HTTP `POST /v1/bridge/heartbeat` (~1.5 s) for `project_root`, tabs, and queued `cmd` objects. Small replies go to `POST /v1/bridge/response`. Every POST carries the random broker-start token discovered from `ws_url`; unauthenticated requests and browser-page CORS access are rejected.
- **Extension WebSocket data plane**: `extension_frame_ws_url` from `GET /v1/bridge` (same host/port as agent `ws_url`, path `/extension/frames`) authenticates one extension Profile. It carries binary **TSH1** screencast packets, direct commands/responses, and acknowledged `network_batch` JSON. Network batches use a capture ID plus monotonic sequence numbers and remain in a bounded extension queue until the broker returns a matching contiguous `network_ack`.
- **Preview capture**: CDP `Page.startScreencast` only (JPEG quality **70**, fit inside **1920×1080**, no upscaling). Frames are sent when the page repaints (scroll, animation, etc.); a static page keeps the last frame and the panel may show **Preview idle**. Locator commands pause screencast briefly, then resume. If screencast cannot start, the extension reports `frame_error` (no HTTP capture fallback). Large JSON frames on `/v1/bridge/response` are deprecated.
- **Agent ↔ bridge**: token-authenticated WebSocket (`ws_url` in `.teshi/cdp-endpoint.json`); ordinary web-page origins are rejected.
- **Desktop**: starts/stops the Python process; polls discovery; `POST /v1/bridge/activate_tab` to switch Chrome tabs.

Chrome may show **“Chrome is being controlled by automated test software”** while screencast runs (debugger attached). This is expected for CDP-based live preview. Keep the active tab on **http(s)**.

The popup **Connect to teshi** button sends one heartbeat immediately and refreshes status text. The green **OK** badge means the last heartbeat succeeded.

## Chrome profile identity, targets, and leases

Each extension installation persists an opaque `extension_instance_id` in that Chromium profile. Heartbeats advertise the optional display label, extension/protocol versions, browser metadata, and every window/tab. Labels, URLs, and titles help selection but are never routing keys. The canonical target is:

```json
{
  "extension_instance_id": "opaque-profile-id",
  "window_id": 7,
  "tab_id": 42
}
```

The broker isolates health, commands, pending requests, preview frames, diagnostics, and an exclusive bounded lease for every extension instance. Different agents may lease different profiles concurrently. A second owner targeting the same profile receives `browser_session_busy`; expired leases recover automatically. Explicit locator and mutation operations require the complete target and lease token. Legacy commands omit them only when exactly one eligible target exists; otherwise they fail with `ambiguous_browser_target` before mutation.

For concurrent work, create one dedicated Chromium profile per agent, install the extension separately, and assign labels such as `agent-a` and `agent-b` in each popup. All profiles use the same loopback broker port.

Network capture is independently keyed by Profile/window/tab/capture ID. One lease owner may capture several explicit tabs in its Profile, while agents holding different Profile leases may capture concurrently. Active-tab changes move only the preview role and do not stop an explicit capture. Tab closure, Profile disconnect, broker replacement, or an external debugger detach ends only the affected capture and reports its termination reason.

## Connect Chrome (default for locators)

Use a **dedicated recording Chrome profile** with real login sessions (SSO, cookies). Install `teshi-bridge` only in that profile so daily browsing and other browser automation tools do not attach to the same debugger target.

Privileged P2 access adds two independent gates: a short-lived Teshi grant bound to the current user, broker, project, caller, and Profile; plus a popup-approved Chromium optional permission where required. Discovery reports public availability only and never exposes grants, lease tokens, Cookie values, or privileged results. Revoking either gate fails closed without disabling P0/P1.

1. Start the dedicated recording Chrome profile manually.
2. Install the unpacked extension from `C:\Program Files\teshi\share\teshi-bridge` (or `extension/teshi-bridge` in repo for development; see [extension README](../extension/teshi-bridge/README.md)).
3. Open the app under test in Chrome and select the target **tab**.
4. Run `teshi browser sessions` or click **Connect Chrome**. Both attach to the same per-user broker; the CLI starts it when absent.
5. Wait until the live stream appears (extension connected; check the status dot tooltip if needed).
6. Select a Gherkin step and run the **playwright-locator** skill in the agent terminal.

While waiting for the extension, the panel shows setup steps only (no stream). Once connected, the panel shows:

- A **tab strip** for tabs in the **current Chrome window** (title + favicon). The active tab is highlighted. Click another debuggable tab to activate it in Chrome; the screencast preview updates on the active http(s) tab when the page repaints.
- Read-only active-tab URL, **Refresh** to restart the stream, and zoom controls (in-panel **Go** is not available in chrome mode).
- If preview is **idle**, interact in Chrome to refresh. If the stream is **disconnected** or shows `frame_error`, check the active tab is http(s), reload the extension, and **Connect Chrome** again.

`chrome://` and extension pages appear in the strip but are not selectable (not CDP-debuggable).

Chrome mode allows agent-driven navigation only for explicit URL steps, for example a Background step that says to open `https://example.com`. Skills should call `teshi browser navigate <url>` only when the URL is present in the step text; they should not invent hidden navigation. Other page changes should happen through confirmed step bindings or direct user action.

Discovery: `GET http://127.0.0.1:17373/v1/bridge` returns broker PID/start identity, protocol, negotiated direct-command transport, versioned `sessions[]`, health, public capabilities/lease metadata, and loopback endpoints. Desktop detach does not stop the shared broker. An incompatible listener is reported and left untouched.
`cdp-endpoint.json` is project compatibility data and includes the shared broker identity and endpoints; project root is request context, not broker ownership.

## External coding agents

The installed `teshi-browser-testing` package contains the `playwright-locator` Skill, compatible extension bundle, MCP metadata, and a machine-readable compatibility declaration. It is installed under `share/teshi-browser-testing` in Teshi release archives/MSI and is also published as a standalone zip. Its Skill may be copied unchanged to a consumer repository's `.agents/skills/playwright-locator` directory.

Use JSON CLI operations or the equivalent local STDIO MCP tools:

```bash
teshi browser sessions
teshi browser tabs --session <instance-id>
teshi browser lease acquire --session <instance-id> --owner agent-a --ttl 60
teshi browser locator --session <instance-id> --window 7 --tab 42 \
  --lease-token <token> --role button --text Save
teshi browser execute --session <instance-id> --window 7 --tab 42 \
  --lease-token <token> --reference @e1 --page-revision <revision> \
  --action click --wait-text Saved
teshi browser network start --session <instance-id> --window 7 --tab 42 \
  --lease-token <token> --host api.example.test \
  --request-body --max-request-body-bytes 262144
teshi browser network list --session <instance-id> --window 7 --tab 42 \
  --lease-token <token>
teshi browser network detail <request-id> --session <instance-id> \
  --window 7 --tab 42 --lease-token <token>
teshi browser network stop --session <instance-id> --window 7 --tab 42 \
  --lease-token <token>
teshi browser lease release --session <instance-id> --lease-token <token>

teshi mcp serve --stdio
```

Locator results include a page-context revision, structured Playwright expression/arguments, frame or shadow context, match count, visibility/actionability, verification state, stability rationale, warnings, and alternatives. The resolver prefers role/name, label, placeholder, project-configured test IDs (default `data-testid`), and stable attributes before CSS fallback. It does not execute or invent a test action. MCP is a same-host process and exposes only profiles registered with that local user's broker.

## Step bindings

Confirmed locators are stored in `.teshi/step-bindings/{feature}.json` and should be committed with the project. The older `{feature}.locators.md` files are deprecated and are no longer written or read by the recording/replay workflow.

Step-binding format 2 can store revision-bound refs and structured candidates; the reader remains compatible with format 1 CSS bindings. Replay routes explicit targets through the canonical action/wait contract. `navigate` remains a first-class setup action and `open_project` calls the SUT runtime API.

`fill` replaces the current value; `type` appends sequential input. Use a separate `press_key` action when Enter is intended.

### Sidecar health

```bash
teshi browser doctor      # JSON report: ok, mode, ws_url, page_url, tcp_reachable, snapshot_ok
teshi browser reconnect   # Spawn detached serve-embedded; refresh cdp-endpoint.json
teshi browser verify --step-line N --selector <css> --action <action> [--value-arg <text>]
```

Before snapshot/replay in embedded mode, run `doctor` (or rely on auto-reconnect). After restarting the SUT or `teshi web`, run `reconnect`.

Set `TESHI_BROWSER_AUTO_RECONNECT=0` to disable automatic reconnect. Set `TESHI_LOCATOR_STRICT=1` to require `browser verify` before `steps propose`.

Values are stored in git as part of the binding. Use placeholders such as `${LOGIN_PW}` for secrets and never commit real passwords, tokens, or private customer data.

## Start Embedded (preview / CI alignment)

Launches **headless Playwright Chromium** with a live JPEG stream in the panel (1920×1080 viewport, ~8 fps). Use for local or staging URLs without the extension.

1. Click **Start Embedded**.
2. Navigate with the in-panel address bar.
3. Record locators the same way via **playwright-locator**.

`cdp-endpoint.json` includes `"mode": "embedded"`.

## Backend mutual exclusion

Embedded remains project-owned. Chrome uses one loopback broker per OS user session, shared by CLI and Desktop, with multiple isolated extension Profiles connected simultaneously.

## CI / replay

Use `teshi browser replay --non-interactive` for CI-style browser setup from confirmed step-bindings. Interactive replay remains the default for terminal agents so each step can be reviewed before execution.

For headless CI without the desktop UI, start the embedded sidecar with:

```bash
teshi browser serve-embedded --navigate http://127.0.0.1:20253
```

This writes `.teshi/cdp-endpoint.json` and keeps Playwright running until Ctrl+C. Pair with `teshi web --no-open` when testing the teshi web UI (see [web-ui-self-test.md](web-ui-self-test.md)).

## Diagnostics

Set `TESHI_BROWSER_DEBUG=1` before starting teshi and running agent commands to persist JSONL diagnostics:

- `.teshi/logs/browser-bridge.log` for Python sidecar and Chrome extension command forwarding.
- `.teshi/logs/cli-browser.log` for `teshi browser` and `teshi steps` command timing, request IDs, and errors.

For snapshot timeouts, reload `teshi-bridge` in `chrome://extensions`, click **Connect Chrome** again, confirm `.teshi/cdp-endpoint.json` has `extension_connected: true`, then rerun `TESHI_BROWSER_DEBUG=1 teshi browser snapshot` and inspect the logs above. Use `teshi browser snapshot --timeout-ms 90000` on heavy pages.

Terminal agents verify locators with `teshi browser execute --selector <css> --action <action>` and `--value-arg <text>` for `fill`, `type`, `assert_text`, `select`, and `press_key` (same flags as `teshi steps propose`). For RVP audit trail, use `teshi browser verify --step-line N ...` (highlight + execute + append to `.teshi/logs/locator-verify.jsonl`).
