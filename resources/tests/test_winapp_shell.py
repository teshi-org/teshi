"""Behavioral tests for Windows shell (notification area) automation.

Covers tray icon enumeration, pid disambiguation through
Shell_NotifyIconGetRect, physical click policy, wait-window visibility
semantics, and the assert_visible false-positive fix. All shell surfaces are
faked; opt-in real-machine acceptance lives in run_winapp_shell_live.py.
"""

import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import winapp_service as service


def tray_button(name, rect, class_name="SystemTray.NormalButton"):
    """One NotifyItemIcon button in a taskbar UIA subtree."""
    return SimpleNamespace(
        ControlTypeName="ButtonControl",
        Name=name,
        ClassName=class_name,
        AutomationId="NotifyItemIcon",
        BoundingRectangle=rect,
        IsOffscreen=False,
        GetChildren=lambda: [],
    )


def container(*children):
    """A non-interactive UIA node carrying children."""
    return SimpleNamespace(
        ControlTypeName="PaneControl",
        Name="",
        ClassName="",
        AutomationId="",
        BoundingRectangle=None,
        GetChildren=lambda: list(children),
    )


class FakeAuto:
    """Minimal uiautomation stand-in mapping handles to control trees."""

    def __init__(self, trees):
        self.trees = trees

    def ControlFromHandle(self, hwnd):
        return self.trees[hwnd]


def user32_mock(**overrides):
    values = {
        "SetThreadDpiAwarenessContext": Mock(return_value=1),
        "FindWindowW": Mock(return_value=0),
        "SendInput": Mock(side_effect=lambda count, inputs, size: count),
        "GetSystemMetrics": Mock(side_effect=lambda index: {76: 0, 77: 0, 78: 2560, 79: 1440}.get(index, 0)),
        "IsWindowVisible": Mock(return_value=True),
        "IsIconic": Mock(return_value=False),
        "GetWindowTextLengthW": Mock(return_value=0),
        "GetWindowTextW": Mock(return_value=0),
        "GetWindowThreadProcessId": Mock(side_effect=lambda hwnd, pid: setattr(pid, "value", 0)),
    }
    values.update(overrides)
    return SimpleNamespace(**values)


class ShellTrayListTests(unittest.TestCase):
    def test_autohidden_taskbar_is_revealed_without_clicking(self):
        def rectangle(hwnd, pointer):
            pointer._obj.left, pointer._obj.top = 0, 1438
            pointer._obj.right, pointer._obj.bottom = 2560, 1498
            return True
        fake = user32_mock(FindWindowW=Mock(return_value=900), GetWindowRect=rectangle)
        with patch.object(service, 'user32', fake), patch.object(service.time, 'sleep'):
            service.reveal_autohidden_taskbar()
        fake.SendInput.assert_called_once()
        event = fake.SendInput.call_args.args[1]._obj
        self.assertEqual(event.mi.dwFlags,
                         service.MOUSEEVENTF_MOVE | service.MOUSEEVENTF_ABSOLUTE | service.MOUSEEVENTF_VIRTUALDESK)

    def test_modern_overflow_button_uses_language_independent_glyph(self):
        glyph = SimpleNamespace(Name='\ue971')
        button = SimpleNamespace(
            ControlTypeName='ButtonControl', AutomationId='SystemTrayIcon',
            ClassName='SystemTray.NormalButton', GetChildren=lambda: [glyph],
            BoundingRectangle=(100, 100, 140, 140),
        )
        with patch.object(service, 'user32', user32_mock()), patch.object(
            service, 'auto', FakeAuto({900: container(button)})
        ), patch.object(service, 'find_taskbar_bridge_hwnd', return_value=900), patch.object(
            service, 'find_overflow_window', return_value=950
        ), patch.object(service, 'send_pointer_click') as click:
            self.assertEqual(service._open_tray_overflow(1), 950)
        click.assert_called_once_with(120, 120)

    def tray_session(self, icons, bridge=900, overflow=None):
        trees = {bridge: container(*icons)}
        if overflow is not None:
            trees[overflow[0]] = container(*overflow[1])
        session = service.WinAppSession(None)
        return session, trees

    def test_tray_list_reports_notification_icons(self):
        icons = [
            tray_button("Enhook", (1719, 1380, 1759, 1440)),
            tray_button("ChatGPT", (1679, 1380, 1719, 1440)),
        ]
        session, trees = self.tray_session(icons)
        with patch.object(service, "auto", FakeAuto(trees)), patch.object(
            service, "find_taskbar_bridge_hwnd", Mock(return_value=900)
        ), patch.object(service, "find_overflow_window", Mock(return_value=None)):
            result = session.shell_tray_list()

        self.assertTrue(result["ok"], result)
        self.assertEqual(len(result["icons"]), 2)
        enhook = result["icons"][0]
        self.assertEqual(enhook["name"], "Enhook")
        self.assertEqual(enhook["area"], "notification")
        self.assertEqual(enhook["rect"], (1719, 1380, 1759, 1440))
        self.assertFalse(result["overflow_open"])

    def test_tray_list_includes_open_overflow_icons(self):
        main = [tray_button("ChatGPT", (1679, 1380, 1719, 1440))]
        overflow_icons = [tray_button("Enhook", (10, 10, 50, 50))]
        session, trees = self.tray_session(main, overflow=(950, overflow_icons))
        with patch.object(service, "auto", FakeAuto(trees)), patch.object(
            service, "find_taskbar_bridge_hwnd", Mock(return_value=900)
        ), patch.object(service, "find_overflow_window", Mock(return_value=950)):
            result = session.shell_tray_list()

        self.assertTrue(result["ok"], result)
        self.assertTrue(result["overflow_open"])
        areas = {icon["name"]: icon["area"] for icon in result["icons"]}
        self.assertEqual(areas["ChatGPT"], "notification")
        self.assertEqual(areas["Enhook"], "overflow")

    def test_tray_list_reports_missing_taskbar_bridge(self):
        session = service.WinAppSession(None)
        with patch.object(service, "auto", FakeAuto({})), patch.object(
            service, "find_taskbar_bridge_hwnd", Mock(return_value=None)
        ):
            result = session.shell_tray_list()

        self.assertFalse(result["ok"])
        self.assertIn("taskbar XAML bridge window not found", result["error"])

    def test_tray_list_relocates_bridge_after_explorer_restart(self):
        # Explorer restart replaces the bridge handle; every call re-resolves
        # it instead of caching, so enumeration keeps working.
        icons = [tray_button("Enhook", (1719, 1380, 1759, 1440))]
        session = service.WinAppSession(None)
        bridges = iter([900, 901])
        trees = {900: container(*icons), 901: container(*icons)}
        with patch.object(service, "auto", FakeAuto(trees)), patch.object(
            service, "find_taskbar_bridge_hwnd", Mock(side_effect=lambda: next(bridges))
        ), patch.object(service, "find_overflow_window", Mock(return_value=None)):
            first = session.shell_tray_list()
            second = session.shell_tray_list()

        self.assertTrue(first["ok"] and second["ok"])
        self.assertEqual(first["icons"][0]["name"], "Enhook")
        self.assertEqual(second["icons"][0]["name"], "Enhook")

    def test_tray_list_does_not_change_attach_session(self):
        session = service.WinAppSession(None)
        session.hwnd = 4242
        session.target_pid = 111
        session.title = "Attached App"
        icons = [tray_button("Enhook", (1719, 1380, 1759, 1440))]
        trees = {900: container(*icons)}
        with patch.object(service, "auto", FakeAuto(trees)), patch.object(
            service, "find_taskbar_bridge_hwnd", Mock(return_value=900)
        ), patch.object(service, "find_overflow_window", Mock(return_value=None)):
            result = session.shell_tray_list()

        self.assertTrue(result["ok"], result)
        self.assertEqual(session.hwnd, 4242)
        self.assertEqual(session.target_pid, 111)
        self.assertEqual(session.title, "Attached App")


class ShellTrayClickTests(unittest.TestCase):
    def click_session(self, icons, bridge=900):
        session = service.WinAppSession(None)
        session.hwnd = 4242
        session.target_pid = 111
        trees = {bridge: container(*icons)}
        return session, trees

    def click(self, session, trees, **payload):
        base = {
            "name": "Enhook",
            "button": "left",
            "count": 2,
            "mode": "auto",
            "timeout_ms": 10000,
        }
        base.update(payload)
        with patch.object(service, "auto", FakeAuto(trees)), patch.object(
            service, "find_taskbar_bridge_hwnd", Mock(return_value=900)
        ), patch.object(service, "find_overflow_window", Mock(return_value=None)), patch.object(
            service, "send_pointer_click"
        ) as inject, patch.object(
            service, "user32", user32_mock()
        ):
            result = session.shell_tray_click(base)
        return result, inject

    def test_tray_click_injects_physical_pointer_at_icon_center(self):
        icons = [tray_button("Enhook", (1719, 1380, 1759, 1440))]
        session, trees = self.click_session(icons)
        result, inject = self.click(session, trees)

        self.assertTrue(result["ok"], result)
        self.assertEqual(result["count"], 2)
        self.assertEqual(result["button"], "left")
        inject.assert_called_once_with(1739, 1410, count=2, button="left")

    def test_tray_click_right_button_single_click(self):
        icons = [tray_button("Enhook", (1719, 1380, 1759, 1440))]
        session, trees = self.click_session(icons)
        result, inject = self.click(session, trees, button="right", count=1)

        self.assertTrue(result["ok"], result)
        inject.assert_called_once_with(1739, 1410, count=1, button="right")

    def test_tray_click_icon_not_found_lists_available(self):
        icons = [tray_button("ChatGPT", (1679, 1380, 1719, 1440))]
        session, trees = self.click_session(icons)
        result, inject = self.click(session, trees)

        self.assertFalse(result["ok"])
        self.assertIn("tray icon not found", result["error"])
        self.assertEqual(result["available_icons"], ["ChatGPT"])
        inject.assert_not_called()

    def test_tray_click_multiple_matches_require_pid(self):
        icons = [
            tray_button("Enhook", (1719, 1380, 1759, 1440)),
            tray_button("Enhook", (1759, 1380, 1799, 1440)),
        ]
        session, trees = self.click_session(icons)
        result, inject = self.click(session, trees)

        self.assertFalse(result["ok"])
        self.assertIn("multiple tray icons named", result["error"])
        self.assertEqual(len(result["candidates"]), 2)
        inject.assert_not_called()

    def test_tray_click_pid_disambiguates_with_scaled_bounds_rounding(self):
        icons = [
            tray_button("Enhook", (1719, 1380, 1759, 1440)),
            tray_button("Enhook", (1759, 1380, 1799, 1440)),
        ]
        session, trees = self.click_session(icons)
        base = {
            "name": "Enhook",
            "pid": 36788,
            "button": "left",
            "count": 2,
            "mode": "auto",
            "timeout_ms": 10000,
        }
        with patch.object(service, "auto", FakeAuto(trees)), patch.object(
            service, "find_taskbar_bridge_hwnd", Mock(return_value=900)
        ), patch.object(service, "find_overflow_window", Mock(return_value=None)), patch.object(
            service, "windows_of_pid", Mock(return_value=[4917820])
        ), patch.object(
            service, "shell_notify_icon_rect",
            Mock(side_effect=lambda hwnd, uid: (1718, 1379, 1758, 1439) if uid == 1 else None),
        ), patch.object(
            service, "send_pointer_click"
        ) as inject, patch.object(
            service, "user32", user32_mock()
        ):
            result = session.shell_tray_click(base)

        self.assertTrue(result["ok"], result)
        self.assertEqual(result["rect"]["left"], 1719)
        inject.assert_called_once_with(1739, 1410, count=2, button="left")

    def test_tray_click_pid_without_icon_fails(self):
        icons = [tray_button("Enhook", (1719, 1380, 1759, 1440))]
        session, trees = self.click_session(icons)
        base = {
            "name": "Enhook",
            "pid": 999999,
            "button": "left",
            "count": 1,
            "mode": "auto",
            "timeout_ms": 10000,
        }
        with patch.object(service, "auto", FakeAuto(trees)), patch.object(
            service, "find_taskbar_bridge_hwnd", Mock(return_value=900)
        ), patch.object(service, "find_overflow_window", Mock(return_value=None)), patch.object(
            service, "windows_of_pid", Mock(return_value=[1])
        ), patch.object(
            service, "shell_notify_icon_rect", Mock(return_value=None)
        ), patch.object(
            service, "send_pointer_click"
        ) as inject, patch.object(
            service, "user32", user32_mock()
        ):
            result = session.shell_tray_click(base)

        self.assertFalse(result["ok"])
        self.assertIn("no notification icon is owned by pid 999999", result["error"])
        inject.assert_not_called()

    def test_tray_click_rejected_in_background_mode(self):
        icons = [tray_button("Enhook", (1719, 1380, 1759, 1440))]
        session, trees = self.click_session(icons)
        result, inject = self.click(session, trees, mode="background")

        self.assertFalse(result["ok"])
        self.assertIn("forbidden in background mode", result["error"])
        inject.assert_not_called()

    def test_tray_click_rejects_unknown_mode_and_button(self):
        icons = [tray_button("Enhook", (1719, 1380, 1759, 1440))]
        session, trees = self.click_session(icons)
        bad_mode, _ = self.click(session, trees, mode="physical")
        self.assertFalse(bad_mode["ok"])
        self.assertIn("invalid mode", bad_mode["error"])

        bad_button, _ = self.click(session, trees, button="middle")
        self.assertFalse(bad_button["ok"])
        self.assertIn("unsupported button", bad_button["error"])

    def test_tray_click_requires_name(self):
        session, trees = self.click_session([])
        result, _ = self.click(session, trees, name="  ")
        self.assertFalse(result["ok"])
        self.assertIn("non-empty name", result["error"])

    def test_tray_click_does_not_change_attach_session(self):
        icons = [tray_button("Enhook", (1719, 1380, 1759, 1440))]
        session, trees = self.click_session(icons)
        result, _ = self.click(session, trees)

        self.assertTrue(result["ok"], result)
        self.assertEqual(session.hwnd, 4242)
        self.assertEqual(session.target_pid, 111)

    def test_tray_click_opens_overflow_when_icon_missing_from_main_area(self):
        main = [tray_button("ChatGPT", (1679, 1380, 1719, 1440))]
        overflow_icons = [tray_button("Enhook", (10, 10, 50, 50))]
        session = service.WinAppSession(None)
        trees = {900: container(*main), 950: container(*overflow_icons)}
        chevron = SimpleNamespace(
            ControlTypeName="ButtonControl",
            Name="",
            ClassName="SystemTray.ChevronButton",
            AutomationId="",
            BoundingRectangle=(1600, 1380, 1640, 1440),
            GetChildren=lambda: [],
        )
        trees[900] = container(*main, chevron)
        base = {
            "name": "Enhook",
            "button": "left",
            "count": 2,
            "mode": "auto",
            "timeout_ms": 10000,
        }
        overflow_calls = {"n": 0}

        def overflow_side_effect():
            # First probe (before opening) finds nothing; afterwards the
            # flyout stays open for the remaining enumeration.
            overflow_calls["n"] += 1
            return None if overflow_calls["n"] == 1 else 950

        with patch.object(service, "auto", FakeAuto(trees)), patch.object(
            service, "find_taskbar_bridge_hwnd", Mock(return_value=900)
        ), patch.object(
            service, "find_overflow_window", Mock(side_effect=overflow_side_effect)
        ), patch.object(
            service, "send_pointer_click"
        ) as inject, patch.object(
            service, "user32", user32_mock()
        ):
            result = session.shell_tray_click(base)

        self.assertTrue(result["ok"], result)
        self.assertEqual(result["area"], "overflow")
        # first click opens the chevron, second clicks the icon itself
        self.assertEqual(inject.call_count, 2)
        inject.assert_any_call(1620, 1410)
        inject.assert_any_call(30, 30, count=2, button="left")


class WaitWindowTests(unittest.TestCase):
    def wait(self, session, states, **payload):
        """states: poll results; the final state repeats until the deadline."""
        base = {"pid": 36788, "state": "visible", "timeout_ms": 10000}
        base.update(payload)
        polls = {"n": 0}

        def next_state(pid):
            index = min(polls["n"], len(states) - 1)
            polls["n"] += 1
            return states[index]

        with patch.object(
            service, "process_window_state", Mock(side_effect=next_state)
        ), patch.object(service.time, "sleep", Mock()):
            return session.wait_window(base)

    def test_wait_window_visible_returns_hwnd(self):
        session = service.WinAppSession(None)
        result = self.wait(
            session,
            [
                {"visible": False, "hwnd": None, "title": None, "windows": []},
                {"visible": True, "hwnd": 6359708, "title": "Enhook", "windows": []},
            ],
        )
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["hwnd"], 6359708)
        self.assertEqual(result["state"], "visible")

    def test_wait_window_detects_hwnd_change_and_refreshes_session(self):
        session = service.WinAppSession(None)
        session.hwnd = 6359708
        session.target_pid = 36788
        session.title = "Enhook old"
        attached = []
        new_window = {"hwnd": 7770001, "title": "Enhook new", "pid": 36788}
        with patch.object(
            service.WinAppSession, "_attach_target",
            lambda self, window: attached.append(window),
        ), patch.object(
            service, "find_window", Mock(return_value=new_window)
        ):
            result = self.wait(
                session,
                [
                    {"visible": False, "hwnd": None, "title": None, "windows": []},
                    {"visible": True, "hwnd": 7770001, "title": "Enhook new", "windows": []},
                ],
            )

        self.assertTrue(result["ok"], result)
        self.assertEqual(result["hwnd"], 7770001)
        self.assertEqual(len(attached), 1)
        self.assertEqual(attached[0]["hwnd"], 7770001)

    def test_wait_window_keeps_session_when_hwnd_unchanged(self):
        session = service.WinAppSession(None)
        session.hwnd = 6359708
        session.target_pid = 36788
        attached = []
        with patch.object(
            service.WinAppSession, "_attach_target",
            lambda self, window: attached.append(window),
        ):
            result = self.wait(
                session,
                [{"visible": True, "hwnd": 6359708, "title": "Enhook", "windows": []}],
            )

        self.assertTrue(result["ok"], result)
        self.assertEqual(attached, [])

    def test_wait_window_does_not_touch_session_of_other_pid(self):
        session = service.WinAppSession(None)
        session.hwnd = 4242
        session.target_pid = 111
        attached = []
        with patch.object(
            service.WinAppSession, "_attach_target",
            lambda self, window: attached.append(window),
        ):
            result = self.wait(
                session,
                [{"visible": True, "hwnd": 7770001, "title": "Other", "windows": []}],
            )

        self.assertTrue(result["ok"], result)
        self.assertEqual(attached, [])

    def test_wait_window_hidden_state(self):
        session = service.WinAppSession(None)
        result = self.wait(
            session,
            [{"visible": True, "hwnd": 6359708, "title": "Enhook", "windows": []}],
            state="hidden",
        )
        self.assertFalse(result["ok"])
        self.assertIn("did not reach state hidden", result["error"])
        self.assertEqual(result["last_state"]["hwnd"], 6359708)

    def test_wait_window_timeout_reports_last_state(self):
        session = service.WinAppSession(None)
        hidden_state = {
            "visible": False,
            "hwnd": None,
            "title": None,
            "windows": [
                {"hwnd": 6359708, "title": "Enhook", "visible": False, "minimized": False}
            ],
        }
        with patch.object(
            service, "process_window_state", Mock(return_value=hidden_state)
        ), patch.object(service.time, "sleep", Mock()):
            result = session.wait_window(
                {"pid": 36788, "state": "visible", "timeout_ms": 500}
            )

        self.assertFalse(result["ok"])
        self.assertIn("did not reach state visible", result["error"])
        self.assertEqual(result["last_state"]["windows"][0]["hwnd"], 6359708)

    def test_wait_window_rejects_non_integer_pid(self):
        session = service.WinAppSession(None)
        result = session.wait_window({"pid": "abc", "state": "visible"})
        self.assertFalse(result["ok"])
        self.assertIn("pid must be an integer", result["error"])


class ProcessWindowStateTests(unittest.TestCase):
    def test_minimized_window_is_not_visible(self):
        fake_user32 = user32_mock(
            IsWindowVisible=Mock(side_effect=lambda hwnd: hwnd != 200),
            IsIconic=Mock(side_effect=lambda hwnd: hwnd == 200),
            GetWindowTextLengthW=Mock(side_effect=lambda hwnd: 6),
            GetWindowTextW=Mock(side_effect=lambda hwnd, buf, n: setattr(buf, "value", "Enhook")),
        )
        with patch.object(service, "user32", fake_user32), patch.object(
            service, "windows_of_pid", Mock(return_value=[100, 200])
        ):
            state = service.process_window_state(36788)

        self.assertTrue(state["visible"])
        self.assertEqual(state["hwnd"], 100)
        minimized = [w for w in state["windows"] if w["hwnd"] == 200][0]
        self.assertTrue(minimized["minimized"])

    def test_hidden_window_reports_not_visible(self):
        fake_user32 = user32_mock(
            IsWindowVisible=Mock(return_value=False),
            IsIconic=Mock(return_value=False),
            GetWindowTextLengthW=Mock(return_value=6),
            GetWindowTextW=Mock(side_effect=lambda hwnd, buf, n: setattr(buf, "value", "Enhook")),
        )
        with patch.object(service, "user32", fake_user32), patch.object(
            service, "windows_of_pid", Mock(return_value=[100])
        ):
            state = service.process_window_state(36788)

        self.assertFalse(state["visible"])
        self.assertIsNone(state["hwnd"])
        self.assertEqual(len(state["windows"]), 1)


class AssertVisibleTests(unittest.TestCase):
    def test_windowless_child_of_minimized_attached_window_fails(self):
        session = self.session()
        session.find_control = Mock(return_value=SimpleNamespace(
            BoundingRectangle=(10, 20, 30, 40), IsOffscreen=False,
            NativeWindowHandle=0,
        ))
        with patch.object(service, 'user32', user32_mock(IsIconic=Mock(return_value=True))):
            result = session.execute({'selector': 'uia:name=Status', 'action': 'assert_visible'})
        self.assertFalse(result['ok'], result)
        self.assertIn('minimized', result['error'])

    def session(self):
        session = service.WinAppSession(None)
        session.hwnd = 123
        session.integrity_guard = Mock(return_value=None)
        session.find_control = Mock()
        return session

    def execute_assert_visible(self, control):
        session = self.session()
        session.find_control = Mock(return_value=control)
        with patch.object(service, "user32", user32_mock()):
            return session.execute(
                {"selector": "uia:name=Status", "action": "assert_visible"}
            )

    def test_hidden_window_with_nonzero_rect_fails(self):
        # A hidden window still exposes non-zero UIA rectangles; the Win32
        # visibility check must reject it instead of trusting the rect.
        control = SimpleNamespace(
            BoundingRectangle=(10, 20, 30, 40),
            IsOffscreen=False,
            NativeWindowHandle=555,
        )
        fake_user32 = user32_mock(IsWindowVisible=Mock(return_value=False))
        session = self.session()
        session.find_control = Mock(return_value=control)
        with patch.object(service, "user32", fake_user32):
            result = session.execute(
                {"selector": "uia:name=Status", "action": "assert_visible"}
            )

        self.assertFalse(result["ok"], result)
        self.assertIn("owning window is hidden", result["error"])

    def test_minimized_window_fails(self):
        control = SimpleNamespace(
            BoundingRectangle=(10, 20, 30, 40),
            IsOffscreen=False,
            NativeWindowHandle=555,
        )
        fake_user32 = user32_mock(IsIconic=Mock(return_value=True))
        session = self.session()
        session.find_control = Mock(return_value=control)
        with patch.object(service, "user32", fake_user32):
            result = session.execute(
                {"selector": "uia:name=Status", "action": "assert_visible"}
            )

        self.assertFalse(result["ok"], result)
        self.assertIn("owning window is minimized", result["error"])

    def test_offscreen_element_fails(self):
        control = SimpleNamespace(
            BoundingRectangle=(10, 20, 30, 40),
            IsOffscreen=True,
        )
        result = self.execute_assert_visible(control)
        self.assertFalse(result["ok"], result)
        self.assertIn("offscreen per UIA IsOffscreen", result["error"])

    def test_unavailable_offscreen_property_fails_without_false_positive(self):
        control = SimpleNamespace(BoundingRectangle=(10, 20, 30, 40))
        result = self.execute_assert_visible(control)
        self.assertFalse(result["ok"], result)
        self.assertIn("cannot determine IsOffscreen", result["error"])

    def test_zero_rect_fails(self):
        control = SimpleNamespace(
            BoundingRectangle=(10, 20, 10, 20),
            IsOffscreen=False,
        )
        result = self.execute_assert_visible(control)
        self.assertFalse(result["ok"], result)
        self.assertIn("no visible UIA bounds", result["error"])

    def test_genuinely_visible_element_passes(self):
        control = SimpleNamespace(
            BoundingRectangle=(10, 20, 30, 40),
            IsOffscreen=False,
            NativeWindowHandle=555,
        )
        fake_user32 = user32_mock()
        session = self.session()
        session.find_control = Mock(return_value=control)
        with patch.object(service, "user32", fake_user32):
            result = session.execute(
                {"selector": "uia:name=Status", "action": "assert_visible"}
            )

        self.assertTrue(result["ok"], result)


if __name__ == "__main__":
    unittest.main()
