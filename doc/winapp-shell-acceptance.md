# Windows shell acceptance

Run the opt-in acceptance on an unlocked Windows 11 desktop using a Python
environment with `uiautomation`, `websockets`, and `Pillow`:

```powershell
python resources/tests/run_winapp_shell_live.py --out .teshi/artifacts/winapp-shell-live
# Include the Rust CLI and WebSocket transport:
python resources/tests/run_winapp_shell_live.py --out .teshi/artifacts/winapp-shell-cli --cli D:/tmp/teshi-winapp-acceptance.exe
```

The runner owns a disposable WinForms application and notification icon. It
injects physical tray input, records mouse events in the application, verifies
window states and UIA assertions, and disposes only its own processes. Reports,
UIA diagnostics, and screenshots stay in the output directory. Use a fresh
directory for each run. Screenshots and shell listings may contain local desktop
content and are not intended for version control.

## Verified on October 20, 2026

The live run at `.teshi/artifacts/winapp-shell-live-20261020-cli` passed 26
checks against the checkout's Python service, with an automatically hidden
Windows 11 taskbar, a 2560 x 1440 desktop, and 125% display scaling:

- Tray enumeration, overflow opening, PID ownership resolution, double-click
  restoration, and right-click hiding. The fixture received both mouse buttons.
- Visible, hidden, minimized, delayed-restoration, and timeout window waits.
- Attachment refresh after a real HWND recreation, followed by a UIA assertion.
- Positive visible assertions and negative assertions for hidden and minimized
  windows, including a child control without its own HWND.
- Background policy rejection of physical tray input.
- Freshly built Rust CLI to private source-service WebSocket transport for
  enumeration, right click, double click, and visible/hidden window waits.

The run exposed three shell compatibility issues: auto-hidden taskbar bounds,
the newer overflow button's language-independent glyph, and one-pixel rounding
differences between Shell and UIA icon bounds. It also exposed a false positive
for children of minimized windows. The implementation and focused regressions
cover these cases.

Focused validation passed 91 Python WinApp tests, 11 Rust WinApp-related tests,
and the workspace Rust formatting check. The CLI built successfully with
`cargo rustc -p teshi-cli --bin teshi --locked -- -o D:/tmp/teshi-winapp-acceptance.exe`;
the separate output preserved an occupied `target/debug/teshi.exe`.

This is real CLI/service/UIA/SendInput acceptance using a disposable WinForms
application. It does not establish managed-runtime packaging, daemon lifecycle,
WGC capture, elevated applications, or universal tray discovery. PID lookup
currently probes conventional notification IDs 0 through 7; GUID-only icons and
other IDs remain outside that lookup contract.
