# Windows shell acceptance

Run the opt-in acceptance on an unlocked Windows 11 desktop using a Python
environment with `uiautomation`, `websockets`, and `Pillow`:

```powershell
python resources/tests/run_winapp_shell_live.py --out .teshi/artifacts/winapp-shell-live
# Verify default cursor/foreground/focus invariants without physical input:
python resources/tests/run_winapp_shell_live.py --background-only --out .teshi/artifacts/winapp-background
# Include the Rust CLI and WebSocket transport:
python resources/tests/run_winapp_shell_live.py --out .teshi/artifacts/winapp-shell-cli --cli D:/tmp/teshi-winapp-acceptance.exe
```

The runner owns a disposable WinForms application and notification icon. It
injects physical tray input, records mouse events in the application, verifies
window states and UIA assertions, and disposes only its own processes. Reports,
UIA diagnostics, and screenshots stay in the output directory. Use a fresh
directory for each run. Screenshots and shell listings may contain local desktop
content and are not intended for version control.

## Input permission review on October 21, 2026

Review base: `origin/dev` at `f61c1b0ba4fc1506a1e08506956e44f1b0c58ac5`.
Implementation worktree: `fix/winapp-input-consent`.

Root causes:

- `WinAppSession.execute` admitted physical `pointer_click` in auto mode and
  activated the target before injection. The action name implicitly granted permission.
- `shell_tray_click` rejected only background. Its default auto path revealed
  an auto-hidden taskbar with SendInput and opened overflow with a real click.
- CLI execute/replay/tray forwarded mode without an independent permission;
  CLI failures also discarded structured response details before printing.
- UIA dispatch accepted unknown return values. PostMessage queue acceptance
  and WM_SETTEXT transport completion were reported as action success.
  Partial message failures could trigger focused fallback and duplicate effects.

The minimum fix uses one request-level permission contract at both Rust and
Python boundaries: only foreground plus the literal boolean permission allows
intrusive input. CLI rejection precedes sidecar startup; replay checks planned
actions before launching the target. Bindings and `--yes` confer no permission.
Unknown action completion stops replay without automatic foreground retry.
Foreground keyboard input also requires confirmed SetFocus before SendKeys.

Current evidence under the worktree's `.teshi/artifacts/`:

- `input-policy-background-final-20261021/report.json`: live WinForms/UIA
  assertions and fill/readback passed. Cursor `(1943,1265)`, foreground HWND
  `66694`, and focused HWND `66694` were identical before and after; guards
  observed zero SetForegroundWindow/SendInput/SetCursorPos/SetFocus/SendKeys calls.
- Earlier `input-policy-live-20261021*` reports retain unsuccessful attempts;
  some samples observed concurrent cursor/foreground changes with no guarded
  API calls. Those samples are not counted as invariant passes.
- Authorized pointer acceptance remains blocked: Windows denied
  SetForegroundWindow. No keyboard/pointer fallback was forced around that denial.
  The subsequent `input-policy-live-20261021-r7` tray sample could not discover
  the fixture icon after opening overflow, so restore business acceptance did
  not pass. These results do not establish a new successful physical-input run.

The live fixture is WinForms. Native WinUI3 custom title bars/caption islands,
elevated targets and application-specific providers still require dedicated
interactive acceptance. A UIA provider or target application can itself focus
windows; the API guards constrain Teshi's input calls, not arbitrary target code.
Do not treat the older physical-input evidence below as validation of this revision.

Compatibility: default pointer/tray scripts and foreground mutation scripts now
need `--mode foreground --allow-interactive-input`. Deploy the matching sidecar
as well as the CLI. Uncertain PostMessage/WM_CLOSE outcomes now fail explicitly;
fill requires value readback, and unknown UIA return values are uncertain.

Final focused validation: 97 Python WinApp tests passed, including six real
CLI/private WebSocket contract tests; 12 Rust WinApp-related tests passed.
`cargo fmt --all --check`, `cargo clippy -p teshi-tui --lib --locked -- -D warnings`,
and `git diff --check` passed. A checkout CLI was built to the worktree's
`.teshi/teshi-input-consent.exe`; no occupied installed CLI was replaced.
Full workspace tests, cross-platform gates and managed-runtime packaging were
not run. These focused passes do not close the interactive acceptance gaps above.

### Subsequent real Enhook validation

Enhook 0.1.56 was tested in fresh isolated profiles using three original Features
and their confirmed bindings from `D:/enhook-source/dev`. Evidence is retained at
`.teshi/artifacts/enhook-input-validation-20261021/acceptance-report.md`; the
corrected physical-input run is `final-acceptance`.

- Original welcome and isolated-profile replays passed, including UIA Skip
  followed by independent Library/Add Software assertions. Welcome, attach and
  snapshots left cursor, foreground and focus unchanged during 10 ms sampling.
- Default pointer/tray requests and foreground pointer without permission were
  rejected with structured errors and zero desktop state changes or guarded
  physical-input/focus API calls.
- Authorized real caption Close hid the window; PID-specific tray double-click
  restored it. Independent window-state and Library/Close visibility assertions
  passed. This supplies real WinUI3 physical business acceptance that the
  earlier WinForms fixture could not establish.
- Skip nevertheless changed foreground/focus to the target, without monitored
  Teshi input/focus API calls. Absolute default non-interference remains open;
  semantic UIA provider/application behavior needs further attribution.
- The close Feature's existing Background failed at line 22 after Settings
  Invoke reported completion but navigation had not occurred. Its manual caption
  visual baseline was not compared; no full manual-scenario pass is claimed.
- Enhook's close handler itself moves the cursor and restores it after an 80 ms
  delay (`MainWindow.xaml.cs`, `HideToTrayAsync`). This is a target application
  bug, as confirmed by the user, and must not be treated as a Teshi workaround.

All owned test processes were cleaned up. Enhook source, bindings, existing
profiles and running user instances were preserved. Earlier single-click tray
timeouts are retained as harness errors: Enhook restores only on double-click.

### UIA focus and navigation correction

The later pattern probes established the previously unassigned focus cause:
Windows UIA enables `IUIAutomation2.AutoSetFocus` by default. The original
client's Skip Invoke changed foreground/focus even with no explicit input API
calls. Replacing the actual shared client and walker with CUIAutomation8 and
reading back AutoSetFocus=false retained successful navigation with zero desktop
state changes in 5 ms sampling. See `pattern-diagnosis.md`, `pattern-on/probe.json`
and `pattern-off3/probe.json` under the Enhook validation artifact directory.

Settings did not use Invoke: its supported LegacyIAccessible default action
returned true without selecting the NavigationViewItem. SelectionItem.Select
changed IsSelected and displayed the settings page. The production bridge now
disables implicit focus before resolving click/fill/select controls, refuses
mutations if this fails, prefers SelectionItem/Toggle over Legacy, and reads
back selection/toggle state. Responses identify the selected input_method.

The final combined run against the rebuilt Enhook worktree passed the original
welcome/profile replays and all four close-to-tray Background steps, including
the formerly failing settings-toggle assertion. Default before/after cursor,
foreground and focus matched. The actual shared UIA client readback was false.
Artifacts: `D:/enhook-source/fix-close-pointer/artifacts/close-pointer/acceptance-final/`.
The final Python suite passed 105 tests, including private CLI transport tests.

Enhook's independent `fix/close-pointer-interference` worktree removes the real
cursor movement/restoration and 80 ms wait. It hides the window and rebuilds the
custom caption instead. Its Debug build and comment checks pass. Physical close
and caption-appearance acceptance of this new Enhook candidate remains blocked
by Windows foreground activation restrictions; the user explicitly chose to
retain this gap rather than manually activate the test window. An occluded crop
is not valid baseline evidence. Earlier physical close/restore passes against the
old Enhook build do not validate this candidate's caption reset.

## Historical verification on October 20, 2026

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
