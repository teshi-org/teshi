# WinUI3 / Native Windows app mode

teshi can expose a native Windows app to terminal agents through the same sidecar pattern used by browser locator recording.

## How WinApp mode communicates

```text
┌──────────────────────┐      WebSocket JSON commands       ┌──────────────────────┐
│  Terminal agent      │ ─────────────────────────────────► │  winapp_service.py   │
│  teshi winapp ...    │ ◄───────────────────────────────── │  127.0.0.1:<port>   │
└──────────────────────┘                                    └──────────┬───────────┘
                                                                         │
                                                                         │ UI Automation
                                                                         ▼
                                                               ┌──────────────────┐
                                                               │  WinUI3 app      │
                                                               └──────────────────┘
                                                                         │
                                                                         │ JPEG frames
                                                                         ▼
                                                               ┌──────────────────┐
                                                               │ teshi preview    │
                                                               └──────────────────┘
```

- **Command plane**: `teshi winapp ...` reads `.teshi/cdp-endpoint.json` and sends JSON commands to the sidecar WebSocket.
- **Preview plane**: the sidecar captures the attached window and broadcasts `frame` messages with base64 JPEG data, which the existing preview panel renders.
- **Element plane**: UI Automation (UIA) provides snapshots, highlighting bounds, and executable actions.

## Start WinApp mode

1. Open a BDD project in teshi Desktop/web.
2. Click **Connect WinUI3 App** in the Target panel.
3. Select a Gherkin step in the left panel.
4. In the terminal, run the `winapp-locator` skill.

If no app is attached yet, list visible windows:

```bash
teshi winapp list-windows
```

Attach explicitly:

```bash
teshi winapp attach --hwnd 123456
teshi winapp attach --title "My App"
teshi winapp attach --process-name MyApp.exe
```

Or launch an executable and wait for its first visible window:

```bash
teshi winapp launch "C:\path\to\MyApp.exe"
```

## GPUI preview prototype

The native and WASM GPUI shells show the same latest-frame preview on their main surface. The prototype automatically starts WinApp mode and attaches to a visible target application window.

For native GPUI, install the project Python dependencies, keep the target application visible, and launch the shell from the project root:

```powershell
uv venv .venv
uv pip install -r python/requirements.txt
cargo run -p teshi-desktop
```

The default target is `TargetApp.exe`. Set `TESHI_WINAPP_PROCESS` to the executable name you want to preview. Set `TESHI_WINAPP_WS_URL` to reuse an already running WinApp sidecar instead of starting one.

For GPUI WASM, build `apps/teshi-web/dist` with `scripts/build-teshi-web.ps1`, then serve it through `teshi web`. The shell starts WinApp mode through `/api/v1/browser/start`, then receives frames from the daemon's same-origin `/api/v1/browser/stream` WebSocket. The Python sidecar stays on the Teshi host's loopback interface, so the preview also works when the page is opened from another machine on the LAN. For diagnostics, `?winapp_ws=<url-encoded-websocket-url>` overrides the proxy endpoint.

The prototype uses screen-rectangle capture. The target application must remain restored, visible, and unobscured. The proxy fixes transport reachability but does not change WebGPU's secure-context requirement: plain HTTP on a LAN address may still fail before the preview opens. Use HTTPS, localhost on the browser machine, or Chromium's development-only `unsafely-treat-insecure-origin-as-secure` setting. When TLS terminates at a reverse proxy, forward `X-Forwarded-Proto: https` so the daemon's same-origin guard accepts the WebSocket upgrade.

## Locator selectors

WinApp mode stores confirmed bindings in `.teshi/step-bindings/{feature}.json` with `strategy: "uia"`.

Selector preference:

1. `uia:automation_id=LoginButton`
2. `uia:control_type=ButtonControl;name=Log in`
3. `uia:name=Log in`
4. `uia:path=0/2/1`

Prefer `AutomationId` whenever the app exposes it. Path selectors are last-resort because UI tree layout can shift between releases.

## Bug-to-regression workflow

1. Describe the bug using [bug-report-template.md](bug-report-template.md).
2. Follow the **bug-to-regression** skill (`.teshi/skills/bug-to-regression/SKILL.md`) to create a `.feature` and bind each step.
3. Use CLI helpers from the project root:

```bash
teshi steps unbound --feature features/my_bug.feature
teshi steps next-unbound --feature features/my_bug.feature
teshi steps select --feature features/my_bug.feature --line 12
teshi winapp replay --feature features/my_bug.feature --yes
teshi export --target behave --feature features/my_bug.feature --out ./tests-e2e
```

See [winui-automation-ids.md](winui-automation-ids.md) for app-side `AutomationId` conventions.

When `.teshi/cdp-endpoint.json` has `"mode": "winapp"`, `teshi run` forwards scenarios to `teshi winapp replay` via the NDJSON runner.

## Supported actions

| Action | UIA behavior |
|--------|--------------|
| `click` | Prefer `InvokePattern`, then UIA click, then center-point click |
| `fill` | Prefer `ValuePattern.SetValue`, then focus + keyboard input |
| `assert_visible` | Check that the resolved element has visible bounds |
| `assert_text` | Compare expected text against `ValuePattern` or `Name` |
| `select` | Prefer `SelectionItemPattern.Select`, then click |
| `press_key` | Focus the element and send keys |

## Dependencies

Project venvs should install:

```bash
pip install -r python/requirements.txt
```

WinApp mode requires `websockets` to start. UI inspection/actions require `uiautomation` and `comtypes`. The screenshot stream uses Pillow `ImageGrab` with `all_screens=True` so targets on secondary monitors are included in the virtual desktop grab. Windows Graphics Capture remains the preferred future backend for unoccluded WinUI3/DirectX windows.

## Limitations

- Target apps running as administrator may require teshi to run at the same integrity level.
- Custom-drawn controls may expose little or no UIA metadata; prefer adding stable `AutomationId` values in the app under test.
- The screenshot path can be affected by occlusion (windows stacked above the target on the same monitor) or protected content.
- Attach only to the app under test. Agents should not guess between multiple plausible native app windows.
