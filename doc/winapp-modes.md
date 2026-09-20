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
2. Start WinApp mode through the installed application's supported control.
3. Select a Gherkin step in the left panel.
4. In the terminal, run the `winapp-regression` skill.

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

## GPUI preview

The native and WASM GPUI shells show the same latest-frame preview on their main surface. WinApp mode prefers Windows Graphics Capture (WGC) for the exact attached HWND and automatically falls back to screen-rectangle ImageGrab when WGC cannot start or stops unexpectedly.

For native GPUI, install the project Python dependencies, keep the target application visible, and launch the shell from the project root:

```powershell
uv venv .venv
uv pip install -r python/requirements.txt
cargo run -p teshi-desktop
```

The default target is `TargetApp.exe`. Set `TESHI_WINAPP_PROCESS` to the executable name you want to preview. Set `TESHI_WINAPP_WS_URL` to reuse an already running WinApp sidecar instead of starting one. The preview status identifies `Windows Graphics Capture` or `ImageGrab fallback` and includes the fallback reason.

For hosted GPUI WASM, run `teshi web` and let the latest `https://teshi.org/app/`
shell start WinApp mode through the authenticated `browser.start` control RPC.
Frames then use the independent `/ws/preview` channel; the Python sidecar URL
remains private on the Teshi host loopback interface. The old `/api/v1/*`
routes remain only for staged migration and legacy clients.

WGC captures the target's composited window surface, so another window may occlude the target without replacing its preview pixels. If ImageGrab fallback is active, the target must remain restored, visible, and unobscured. Neither backend can capture protected content, and a closed HWND produces a stream error rather than falling back to unrelated screen pixels. The proxy fixes transport reachability but does not change WebGPU's secure-context requirement: plain HTTP on a LAN address may still fail before the preview opens. Use HTTPS, localhost on the browser machine, or Chromium's development-only `unsafely-treat-insecure-origin-as-secure` setting. When TLS terminates at a reverse proxy, forward `X-Forwarded-Proto: https` so the daemon's same-origin guard accepts the WebSocket upgrade.

## Locator selectors

WinApp mode stores confirmed bindings in `.teshi/step-bindings/{feature}.json` with `strategy: "uia"`.

Selector preference:

1. `uia:automation_id=LoginButton`
2. `uia:control_type=ButtonControl;name=Log in`
3. `uia:name=Log in`
4. `uia:path=0/2/1`

Prefer `AutomationId` whenever the app exposes it. Path selectors are last-resort because UI tree layout can shift between releases.

## WinApp regression workflow

1. Describe the bug using [bug-report-template.md](bug-report-template.md).
2. Follow the **winapp-regression** skill (`skills/winapp-regression/SKILL.md`) to create a `.feature` and bind each step.
3. Use CLI helpers from the project root:

```bash
teshi steps unbound --feature features/my_bug.feature
teshi steps next-unbound --feature features/my_bug.feature
teshi steps select --feature features/my_bug.feature --line 12
teshi winapp replay --feature features/my_bug.feature --yes
teshi export --target behave --feature features/my_bug.feature --out ./tests-e2e
```

`pointer_click` is currently a native WinApp replay action and is not
supported by the behave exporter. Exporting a binding that uses it fails with
`unsupported export action: pointer_click`; keep that scenario on native replay
or add a custom behave step definition.

See [winui-automation-ids.md](winui-automation-ids.md) for app-side `AutomationId` conventions.

When `.teshi/cdp-endpoint.json` has `"mode": "winapp"`, `teshi run` forwards scenarios to `teshi winapp replay` via the NDJSON runner.

## Supported actions

| Action | UIA behavior |
|--------|--------------|
| `click` | Prefer `InvokePattern`, then UIA click, then center-point click; does not guarantee real pointer hover or pressed state |
| `pointer_click` | Foreground-only real pointer move to the element center followed by a Win32 `SendInput` left-click |
| `fill` | Prefer `ValuePattern.SetValue`, then focus + keyboard input |
| `assert_visible` | Check that the resolved element has visible bounds |
| `assert_not_exists` | Pass only when the UIA selector matches no element, including hidden elements |
| `assert_text` | Compare expected text against `ValuePattern` or `Name` |
| `assert_screenshot` | Compare lossless RGB pixels of one visible interactive UIA element with a PNG baseline |
| `select` | Prefer `SelectionItemPattern.Select`, then click |
| `press_key` | Focus the element and send keys |

Use `pointer_click` when the control depends on real pointer input, such as
hover/pressed state, a WinUI3 custom title bar or caption island, non-client
area interaction, or pointer messages. It moves the system pointer and brings
the attached window to the foreground. `click` remains the UIA-activation-first
choice for ordinary stable automation.

```powershell
teshi winapp execute --selector "uia:control_type=ButtonControl;name=Close" --action pointer_click
```

## Element screenshots and visual assertions

```powershell
teshi winapp screenshot --selector "uia:control_type=ButtonControl;name=Close" --out artifacts/close-normal.png
teshi winapp assert-screenshot --selector "uia:control_type=ButtonControl;name=Close" --baseline artifacts/close-normal.png --diff-out artifacts/close-diff.png --pixel-tolerance 8
```

Both commands return JSON on operational success and failure. A failed visual assertion returns `ok: false` and a nonzero CLI exit code. Paths are relative to the project root unless absolute. Capture does not activate the target or change its hover/pressed state.

Screenshot resolution uses the UIA snapshot's visible interactive elements, independently of the existing action resolver. Offscreen elements are excluded. Multiple visible matches, incomplete snapshots, missing bounds, window/element movement during capture, and unverifiable coordinate mappings fail explicitly.

`bounds.x`, `bounds.y`, `bounds.width`, and `bounds.height` describe physical screen pixels; `dpi` comes from `GetDpiForWindow`. Visual commands temporarily set per-monitor thread DPI awareness. WGC uses the DWM extended frame rectangle, including caption islands and excluding invisible resize borders. ImageGrab uses the physical `GetWindowRect` screen rectangle. The element crop subtracts the captured rectangle's screen origin without scaling. Frame dimensions must exactly match that rectangle, and the complete element must fit inside it. Negative monitor origins are supported; out-of-range crops are rejected.

The preview remains JPEG. Visual commands retain RGB directly from WGC's BGRA frame or ImageGrab's RGB image and save PNG; they never decode preview JPEG. A WGC visual capture restarts the existing backend to obtain a fresh first frame even when the window is static. WGC keeps `cursor_capture=False`; ImageGrab uses its screen-copy path without cursor composition. The ImageGrab fallback requires the target to remain visible and unobscured.

`pixel-tolerance=8` allows an absolute difference of **at most 8 in every RGB channel**. A pixel counts once as changed if any channel exceeds 8. Dimensions must match regardless of tolerance. Generated baselines embed `teshi_bounds` and `teshi_dpi` PNG text metadata; changes to the recorded UIA width or height also fail. Position changes alone do not fail. External PNG baselines without metadata use their pixel dimensions as the size contract.

Comparison failures include `changed_pixels`, `baseline_dimensions`, `actual_dimensions`, `bounds`, `dpi`, and `diff_out`. The diff is a lossless PNG with changed/missing pixels marked magenta and unchanged pixels black; different sizes use a canvas large enough for both images. A successful comparison removes an existing diff at the requested output path. The baseline and diff paths must differ.

For step bindings, keep the existing schema:

```json
{
  "strategy": "uia",
  "value": "uia:control_type=ButtonControl;name=Close",
  "action": "assert_screenshot",
  "value_arg": "test/visual-baselines/close-normal.png"
}
```

WinApp replay uses tolerance 8 and writes failures under `.teshi/artifacts/visual/<sanitized-feature>-L<line>-diff.png`. The assertion response is printed before replay reports the failing step. Existing replay JPEG evidence remains separate from visual comparison.

Managed runtime v3 packages real pointer clicking and the existing visual
capability. The install path and release archive are versioned separately from
the Teshi application; v2 manifests cannot satisfy the v3 requirement. Release
builds embed the v3 archive URL, hash, and size. Restart an already-running
older daemon/sidecar after upgrading so it loads the new runtime.

Run the window-independent capture, comparison, selector, CLI, and replay tests after building the CLI:

```powershell
cargo build -p teshi-cli --locked
python -m pip install Pillow websockets
python -m unittest discover -s resources/tests -p 'test_winapp*.py'
```

## Dependencies

Project venvs should install:

```bash
pip install -r python/requirements.txt
```

WinApp mode requires `websockets` to start. UI inspection/actions require `uiautomation` and `comtypes`. On Windows x64 with Python 3.9+, `windows-capture==2.0.1` supplies the preferred WGC backend; its wheel depends on NumPy and OpenCV. WGC HWND interop requires Windows 10 version 1903 (build 18362) or newer. Pillow remains required for JPEG encoding and the ImageGrab fallback. Non-Windows development environments do not install the WGC package.

## Limitations

- Target apps running as administrator may require teshi to run at the same integrity level.
- Custom-drawn controls may expose little or no UIA metadata; prefer adding stable `AutomationId` values in the app under test.
- WGC cannot capture protected content. ImageGrab fallback is additionally affected by occlusion and minimized windows.
- Independent top-level popup windows are not merged into the attached main HWND's WGC stream.
- Attach only to the app under test. Agents should not guess between multiple plausible native app windows.
