"""Opt-in interactive Windows acceptance; never uses a user's application.

Run with a Python environment containing the WinApp service dependencies.
The JSON report and screenshots are written to --out; physical tray clicks
require an unlocked interactive Windows 11 desktop.
"""
import argparse
import json
import subprocess
import sys
import time
import threading
import os
import secrets
import ctypes
from ctypes import wintypes
from unittest.mock import patch
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import winapp_service as service
from PIL import ImageGrab


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--background-only', action='store_true', help='Run default desktop invariants without physical input')
    parser.add_argument('--cli', type=Path, help='Optional freshly built CLI for transport acceptance')
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    report = []
    process = subprocess.Popen([
        'powershell', '-NoProfile', '-WindowStyle', 'Hidden', '-File',
        str(Path(__file__).with_name('winapp_shell_fixture.ps1')),
        '-StateDir', str(out),
    ])
    session = service.WinAppSession(None)

    def check(name, result, expected=True):
        report.append({'check': name, 'result': result})
        (out / 'report.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
        if result.get('ok') != expected:
            with service.physical_coordinates():
                ImageGrab.grab().save(out / 'failure.png')
                print('physical screen', service.user32.GetSystemMetrics(0), service.user32.GetSystemMetrics(1))
                print('pid rect', service.notify_icon_rect_for_pid(process.pid))
            hwnd = service.find_overflow_window()
            if not hwnd:
                hwnd = service.find_taskbar_bridge_hwnd()
            if hwnd:
                nodes = []
                def walk(control, depth=0):
                    if depth > 12:
                        return
                    nodes.append([depth, control.ControlTypeName, control.Name, control.AutomationId, control.ClassName, service.rect_to_dict(control.BoundingRectangle)])
                    for child in control.GetChildren():
                        walk(child, depth + 1)
                walk(service.auto.ControlFromHandle(hwnd))
                (out / 'overflow-tree.json').write_text(json.dumps(nodes, indent=2), encoding='utf-8')
        assert result.get('ok') == expected, (name, result)

    def command(value):
        (out / 'command.txt').write_text(value, encoding='utf-8')
        time.sleep(0.5)

    def wait(state, timeout=3000):
        return session.wait_window({'pid': process.pid, 'state': state, 'timeout_ms': timeout})

    def visible():
        return session.execute({'selector': 'uia:name=Teshi visibility marker', 'action': 'assert_visible'})

    try:
        check('initial visible window', wait('visible', 15000))
        time.sleep(3)  # Explorer publishes newly registered icons asynchronously.
        session._attach_target(service.find_window(hwnd=service.process_window_state(process.pid)['hwnd']))
        check('visible marker', visible())
        check('tray enumeration', service.shell_tray_icons())
        class GuiThreadInfo(ctypes.Structure):
            _fields_ = [('cbSize', wintypes.DWORD), ('flags', wintypes.DWORD),
                        ('hwndActive', wintypes.HWND), ('hwndFocus', wintypes.HWND),
                        ('hwndCapture', wintypes.HWND), ('hwndMenuOwner', wintypes.HWND),
                        ('hwndMoveSize', wintypes.HWND), ('hwndCaret', wintypes.HWND),
                        ('rcCaret', wintypes.RECT)]

        def desktop_state():
            point = wintypes.POINT()
            assert service.user32.GetCursorPos(ctypes.byref(point))
            foreground = service.user32.GetForegroundWindow()
            thread = service.user32.GetWindowThreadProcessId(foreground, None)
            info = GuiThreadInfo(cbSize=ctypes.sizeof(GuiThreadInfo))
            assert service.user32.GetGUIThreadInfo(thread, ctypes.byref(info))
            return {'focus': int(info.hwndFocus or 0), 'cursor': [point.x, point.y],
                    'foreground': int(service.user32.GetForegroundWindow()),
                    'foreground_title': service.get_window_title(service.user32.GetForegroundWindow())}

        calls = []
        real_user32 = service.user32
        class GuardedUser32:
            def __getattr__(self, name):
                if name in ('SetForegroundWindow', 'SendInput', 'SetFocus', 'SetCursorPos'):
                    def forbidden(*args, **kwargs):
                        calls.append(name)
                        raise AssertionError(f'Unauthorized desktop input: {name}')
                    return forbidden
                return getattr(real_user32, name)
        def forbidden_keys(*args, **kwargs):
            calls.append('SendKeys/SetFocus')
            raise AssertionError('Unauthorized keyboard focus/input')
        time.sleep(2)  # Let the fixture's initial Show/activation finish before sampling.
        before = desktop_state()
        with patch.object(service, 'user32', GuardedUser32()), patch.object(
            service.auto, 'SendKeys', forbidden_keys
        ), patch.object(service.auto.Control, 'SetFocus', forbidden_keys):
            for mode in ('auto', 'background', 'foreground'):
                for permission in (False, True):
                    if mode == 'foreground' and permission:
                        continue
                    denied = session.execute({'selector': 'uia:name=Teshi visibility marker',
                        'action': 'pointer_click', 'mode': mode, 'allow_interactive_input': permission})
                    check(f'pointer policy {mode}/{permission}',
                          {'ok': denied.get('code') == 'requires_interactive_input', 'response': denied})
                    denied = session.shell_tray_click({'name': 'Teshi Shell Acceptance',
                        'mode': mode, 'allow_interactive_input': permission})
                    check(f'tray policy {mode}/{permission}',
                          {'ok': denied.get('code') == 'requires_interactive_input', 'response': denied})
            check('background marker read', visible())
            report.append({'check': 'state after read', 'state': desktop_state()})
            check('background fill readback', session.execute({'selector': 'uia:name=Acceptance input',
                  'action': 'fill', 'value': 'Verified background value'}))
            report.append({'check': 'state after fill', 'state': desktop_state()})
            check('background fill business assertion', session.execute({'selector': 'uia:name=Acceptance input',
                  'action': 'assert_text', 'value': 'Verified background value'}))
        after = desktop_state()
        check('default desktop unchanged with zero intrusive API calls',
              {'ok': before == after and not calls, 'before': before, 'after': after, 'calls': calls})
        if args.background_only:
            print(f'PASS: default desktop invariants; evidence: {out}')
            return
        pointer = session.execute({'selector': 'uia:name=Acceptance pointer',
            'action': 'pointer_click', 'mode': 'foreground', 'allow_interactive_input': True})
        blocked_pointer = not pointer.get('ok') and 'SetForegroundWindow failed' in pointer.get('error', '')
        if blocked_pointer:
            report.append({'check': 'authorized pointer business acceptance', 'status': 'blocked',
                           'result': pointer, 'reason': 'Windows denied foreground activation'})
        else:
            check('authorized physical button input', pointer)
            deadline = time.monotonic() + 3
            while not (out / 'pointer.txt').exists() and time.monotonic() < deadline:
                time.sleep(0.05)
            check('physical button business event', {'ok': (out / 'pointer.txt').exists()})
        command('hide')
        check('hidden window wait', wait('hidden'))
        check('hidden marker rejected', visible(), False)
        check('visible wait times out with last state', wait('visible', 500), False)
        check('background physical input rejected', session.shell_tray_click({
            'name': 'Teshi Shell Acceptance', 'mode': 'background'}), False)
        check('double click restores window', session.shell_tray_click({
            'name': 'Teshi Shell Acceptance', 'pid': process.pid, 'count': 2, 'mode': 'foreground', 'allow_interactive_input': True}))
        check('restored window wait', wait('visible'))
        check('restored marker', visible())
        old_hwnd = session.hwnd
        command('recreate')
        check('recreated window found', wait('visible'))
        check('attachment refreshed after HWND recreation', {
            'ok': session.hwnd != old_hwnd,
            'old_hwnd': old_hwnd, 'new_hwnd': session.hwnd})
        check('marker after HWND recreation', visible())
        ImageGrab.grab().save(out / 'restored.png')
        check('tray enumeration after opening overflow', service.shell_tray_icons())
        command('minimize')
        check('minimized window counts hidden', wait('hidden'))
        check('minimized marker rejected', visible(), False)
        command('show')
        check('right click injected', session.shell_tray_click({
            'name': 'Teshi Shell Acceptance', 'pid': process.pid, 'button': 'right', 'mode': 'foreground', 'allow_interactive_input': True}))
        check('right click hides application', wait('hidden'))
        events = (out / 'clicks.txt').read_text(encoding='utf-8-sig')
        check('application received real left and right mouse events', {
            'ok': 'Left' in events and 'Right' in events, 'events': events})
        timer = threading.Timer(0.7, command, args=('show',))
        timer.start()
        try:
            check('wait observes delayed restoration', wait('visible'))
        finally:
            timer.join()
        if args.cli:
            # A private endpoint proves the checkout CLI reaches this source
            # service without replacing a running user's daemon or runtime.
            server = subprocess.Popen([
                sys.executable, str(Path(service.__file__).resolve()), '--port', '0',
                '--project-root', str(out), '--auth-token', secrets.token_urlsafe(32),
            ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                endpoint = out / '.teshi' / 'cdp-endpoint.json'
                deadline = time.monotonic() + 10
                while not endpoint.exists() and time.monotonic() < deadline:
                    time.sleep(0.1)
                assert endpoint.exists(), 'Private service did not start'
                env = dict(os.environ, TESHI_APP_DATA_DIR=str(out / 'app-data'))
                def cli(*arguments):
                    completed = subprocess.run([
                        str(args.cli.resolve()), 'winapp', *arguments,
                    ], cwd=out, env=env, capture_output=True, text=True, timeout=25,
                       encoding='utf-8', errors='replace')
                    return {'ok': completed.returncode == 0, 'stdout': completed.stdout,
                            'stderr': completed.stderr, 'exit_code': completed.returncode}
                check('CLI tray list transport', cli('shell', 'tray', 'list'))
                check('CLI visible wait transport', cli('wait-window', '--pid', str(process.pid), '--visible'))
                check('CLI right click transport', cli('shell', 'tray', 'click', '--name',
                      'Teshi Shell Acceptance', '--pid', str(process.pid), '--button', 'right', '--mode', 'foreground', '--allow-interactive-input'))
                check('CLI hidden wait transport', cli('wait-window', '--pid', str(process.pid), '--hidden'))
                check('CLI double click transport', cli('shell', 'tray', 'click', '--name',
                      'Teshi Shell Acceptance', '--pid', str(process.pid), '--count', '2', '--mode', 'foreground', '--allow-interactive-input'))
                check('CLI restored wait transport', cli('wait-window', '--pid', str(process.pid), '--visible'))
            finally:
                server.terminate()
                server.wait(timeout=5)
        print(f'{"PARTIAL" if blocked_pointer else "PASS"}: {len(report)} live checks; evidence: {out}')
        if blocked_pointer:
            raise SystemExit(2)
    finally:
        session._stop_capture_backend()
        command('quit')
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=5)


if __name__ == '__main__':
    main()
