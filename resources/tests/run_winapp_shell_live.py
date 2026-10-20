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
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import winapp_service as service
from PIL import ImageGrab


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--out', type=Path, required=True)
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
        command('hide')
        check('hidden window wait', wait('hidden'))
        check('hidden marker rejected', visible(), False)
        check('visible wait times out with last state', wait('visible', 500), False)
        check('background physical input rejected', session.shell_tray_click({
            'name': 'Teshi Shell Acceptance', 'mode': 'background'}), False)
        check('double click restores window', session.shell_tray_click({
            'name': 'Teshi Shell Acceptance', 'pid': process.pid, 'count': 2}))
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
            'name': 'Teshi Shell Acceptance', 'pid': process.pid, 'button': 'right'}))
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
                      'Teshi Shell Acceptance', '--pid', str(process.pid), '--button', 'right'))
                check('CLI hidden wait transport', cli('wait-window', '--pid', str(process.pid), '--hidden'))
                check('CLI double click transport', cli('shell', 'tray', 'click', '--name',
                      'Teshi Shell Acceptance', '--pid', str(process.pid), '--count', '2'))
                check('CLI restored wait transport', cli('wait-window', '--pid', str(process.pid), '--visible'))
            finally:
                server.terminate()
                server.wait(timeout=5)
        print(f'PASS: {len(report)} live checks; evidence: {out}')
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
