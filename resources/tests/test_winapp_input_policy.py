"""Behavioral boundaries for non-intrusive WinApp action dispatch."""

import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import winapp_service as service


def unavailable_pattern():
    raise RuntimeError("pattern unavailable")


class InputPolicyTests(unittest.TestCase):
    def session(self, control=None):
        session = service.WinAppSession(None)
        session.hwnd = 123
        session.integrity_guard = Mock(return_value=None)
        if control is not None:
            session.find_control = Mock(return_value=control)
        return session

    @staticmethod
    def user32_mock(**overrides):
        values = {
            "SetForegroundWindow": Mock(return_value=True),
            "SetThreadDpiAwarenessContext": Mock(return_value=1),
            "ScreenToClient": Mock(return_value=True),
            "PostMessageW": Mock(return_value=True),
            "SendMessageTimeoutW": Mock(return_value=1),
            "MapVirtualKeyW": Mock(return_value=1),
            "VkKeyScanW": Mock(side_effect=lambda char: ord(char)),
        }
        values.update(overrides)
        return SimpleNamespace(**values)

    def test_read_assertions_never_activate_foreground(self):
        value_pattern = SimpleNamespace(Value="Ready")
        control = SimpleNamespace(
            BoundingRectangle=(10, 20, 30, 40),
            GetValuePattern=lambda: value_pattern,
        )
        fake_user32 = self.user32_mock()
        session = self.session(control)

        with patch.object(service, "user32", fake_user32):
            visible = session.execute(
                {"selector": "uia:name=Status", "action": "assert_visible"}
            )
            text = session.execute(
                {
                    "selector": "uia:name=Status",
                    "action": "assert_text",
                    "value": "Ready",
                    "mode": "foreground",
                }
            )
            session.control_exists = Mock(return_value=False)
            absent = session.execute(
                {
                    "selector": "uia:name=Missing",
                    "action": "assert_not_exists",
                    "mode": "foreground",
                }
            )

        self.assertTrue(visible["ok"], visible)
        self.assertTrue(text["ok"], text)
        self.assertTrue(absent["ok"], absent)
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_click_invoke_success_never_activates_foreground(self):
        invoke = Mock()
        control = SimpleNamespace(
            GetInvokePattern=lambda: SimpleNamespace(Invoke=invoke),
        )
        fake_user32 = self.user32_mock()
        session = self.session(control)

        with patch.object(service, "user32", fake_user32):
            result = session.execute(
                {
                    "selector": "uia:name=Save",
                    "action": "click",
                    "mode": "foreground",
                }
            )

        self.assertTrue(result["ok"], result)
        invoke.assert_called_once_with()
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_fill_value_pattern_success_never_focuses_or_activates(self):
        set_value = Mock()
        control = SimpleNamespace(
            GetValuePattern=lambda: SimpleNamespace(SetValue=set_value),
            SetFocus=Mock(),
        )
        fake_user32 = self.user32_mock()
        session = self.session(control)

        with patch.object(service, "user32", fake_user32):
            result = session.execute(
                {
                    "selector": "uia:automation_id=Name",
                    "action": "fill",
                    "value": "Ada",
                    "mode": "foreground",
                }
            )

        self.assertTrue(result["ok"], result)
        set_value.assert_called_once_with("Ada")
        control.SetFocus.assert_not_called()
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_fill_without_control_hwnd_uses_explicit_foreground_fallback(self):
        control = SimpleNamespace(
            GetValuePattern=unavailable_pattern,
            SetFocus=Mock(),
        )
        fake_user32 = self.user32_mock()
        fake_auto = SimpleNamespace(SendKeys=Mock())
        session = self.session(control)

        with patch.object(service, "user32", fake_user32), patch.object(
            service, "auto", fake_auto
        ):
            result = session.execute(
                {
                    "selector": "uia:automation_id=Name",
                    "action": "fill",
                    "value": "Ada",
                    "mode": "foreground",
                }
            )

        self.assertTrue(result, result)
        fake_user32.SendMessageTimeoutW.assert_not_called()
        fake_user32.SetForegroundWindow.assert_called_once_with(123)
        control.SetFocus.assert_called_once_with()
        self.assertEqual(fake_auto.SendKeys.call_count, 2)

    def test_fill_without_control_hwnd_fails_closed_in_auto_mode(self):
        control = SimpleNamespace(GetValuePattern=unavailable_pattern)
        fake_user32 = self.user32_mock()
        session = self.session(control)

        with patch.object(service, "user32", fake_user32):
            result = session.execute(
                {
                    "selector": "uia:automation_id=Name",
                    "action": "fill",
                    "value": "Ada",
                }
            )

        self.assertFalse(result["ok"], result)
        self.assertIn("no control HWND for background fill", result["error"])
        fake_user32.SendMessageTimeoutW.assert_not_called()
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_select_pattern_success_never_activates_foreground(self):
        select = Mock()
        control = SimpleNamespace(
            GetSelectionItemPattern=lambda: SimpleNamespace(Select=select),
        )
        fake_user32 = self.user32_mock()
        session = self.session(control)

        with patch.object(service, "user32", fake_user32):
            result = session.execute(
                {
                    "selector": "uia:name=Choice",
                    "action": "select",
                    "mode": "foreground",
                }
            )

        self.assertTrue(result["ok"], result)
        select.assert_called_once_with()
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_pointer_click_activates_and_injects_real_pointer_in_auto(self):
        fake_user32 = self.user32_mock()
        session = self.session()
        session.find_visible_interactive_control = Mock(
            return_value=(object(), {"left": 10, "top": 20, "width": 20, "height": 10})
        )

        with patch.object(service, "user32", fake_user32), patch.object(
            service, "send_pointer_click"
        ) as inject:
            result = session.execute(
                {"selector": "uia:name=Close", "action": "pointer_click"}
            )

        self.assertTrue(result["ok"], result)
        fake_user32.SetForegroundWindow.assert_called_once_with(123)
        inject.assert_called_once_with(20, 25)

    def test_pointer_click_background_fails_without_activation_or_injection(self):
        fake_user32 = self.user32_mock()
        session = self.session()

        with patch.object(service, "user32", fake_user32), patch.object(
            service, "send_pointer_click"
        ) as inject:
            result = session.execute(
                {
                    "selector": "uia:name=Close",
                    "action": "pointer_click",
                    "mode": "background",
                }
            )

        self.assertFalse(result["ok"], result)
        self.assertIn("forbidden in background mode", result["error"])
        fake_user32.SetForegroundWindow.assert_not_called()
        inject.assert_not_called()

    def test_background_click_uses_messages_but_never_send_input(self):
        control = SimpleNamespace(
            GetInvokePattern=unavailable_pattern,
            GetLegacyIAccessiblePattern=unavailable_pattern,
            GetTogglePattern=unavailable_pattern,
            GetSelectionItemPattern=unavailable_pattern,
            BoundingRectangle=(10, 20, 30, 40),
            NativeWindowHandle=456,
        )
        fake_user32 = self.user32_mock()
        session = self.session(control)

        with patch.object(service, "user32", fake_user32), patch.object(
            service, "send_pointer_click"
        ) as inject:
            result = session.execute(
                {
                    "selector": "uia:name=Legacy",
                    "action": "click",
                    "mode": "background",
                }
            )

        self.assertTrue(result["ok"], result)
        self.assertEqual(fake_user32.PostMessageW.call_count, 2)
        fake_user32.SetForegroundWindow.assert_not_called()
        inject.assert_not_called()

    def test_press_key_keeps_brace_modifier_held_for_following_key(self):
        control = SimpleNamespace(NativeWindowHandle=456)
        fake_user32 = self.user32_mock()
        session = self.session(control)

        with patch.object(service, "user32", fake_user32):
            result = session.execute(
                {
                    "selector": "uia:name=Editor",
                    "action": "press_key",
                    "value": "{Ctrl}a",
                }
            )

        self.assertTrue(result, result)
        posted = [
            (call.args[1], call.args[2])
            for call in fake_user32.PostMessageW.call_args_list
        ]
        self.assertEqual(
            posted,
            [
                (service.WM_KEYDOWN, service.VK_CONTROL),
                (service.WM_KEYDOWN, ord("a")),
                (service.WM_CHAR, ord("a")),
                (service.WM_KEYUP, ord("a")),
                (service.WM_KEYUP, service.VK_CONTROL),
            ],
        )
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_failed_background_press_key_fails_closed_in_auto_mode(self):
        control = SimpleNamespace(NativeWindowHandle=456)
        fake_user32 = self.user32_mock(PostMessageW=Mock(return_value=False))
        session = self.session(control)

        with patch.object(service, "user32", fake_user32):
            result = session.execute(
                {
                    "selector": "uia:name=Editor",
                    "action": "press_key",
                    "value": "a",
                }
            )

        self.assertFalse(result["ok"], result)
        self.assertIn("PostMessageW failed", result["error"])
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_failed_background_press_key_uses_explicit_foreground_fallback(self):
        control = SimpleNamespace(NativeWindowHandle=456, SetFocus=Mock())
        fake_user32 = self.user32_mock(PostMessageW=Mock(return_value=False))
        fake_auto = SimpleNamespace(SendKeys=Mock())
        session = self.session(control)

        with patch.object(service, "user32", fake_user32), patch.object(
            service, "auto", fake_auto
        ):
            result = session.execute(
                {
                    "selector": "uia:name=Editor",
                    "action": "press_key",
                    "value": "{Ctrl}a",
                    "mode": "foreground",
                }
            )

        self.assertTrue(result, result)
        fake_user32.SetForegroundWindow.assert_called_once_with(123)
        control.SetFocus.assert_called_once_with()
        fake_auto.SendKeys.assert_called_once_with("{Ctrl}a", waitTime=0.05)

    def test_auto_does_not_escalate_failed_click_to_foreground(self):
        control = SimpleNamespace(
            GetInvokePattern=unavailable_pattern,
            GetLegacyIAccessiblePattern=unavailable_pattern,
            GetTogglePattern=unavailable_pattern,
            GetSelectionItemPattern=unavailable_pattern,
            BoundingRectangle=None,
        )
        fake_user32 = self.user32_mock()
        session = self.session(control)

        with patch.object(service, "user32", fake_user32):
            result = session.execute(
                {"selector": "uia:name=Custom", "action": "click"}
            )

        self.assertFalse(result["ok"], result)
        self.assertIn("auto mode does not implicitly grant foreground input", result["error"])
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_replay_style_sequence_activates_only_pointer_step(self):
        control = SimpleNamespace(BoundingRectangle=(0, 0, 10, 10))
        fake_user32 = self.user32_mock()
        session = self.session(control)
        session.find_visible_interactive_control = Mock(
            return_value=(control, {"left": 0, "top": 0, "width": 10, "height": 10})
        )

        with patch.object(service, "user32", fake_user32), patch.object(
            service, "send_pointer_click"
        ) as inject:
            assertion = session.execute(
                {"selector": "uia:name=Status", "action": "assert_visible", "mode": "auto"}
            )
            pointer = session.execute(
                {"selector": "uia:name=Close", "action": "pointer_click", "mode": "auto"}
            )

        self.assertTrue(assertion["ok"], assertion)
        self.assertTrue(pointer["ok"], pointer)
        fake_user32.SetForegroundWindow.assert_called_once_with(123)
        inject.assert_called_once_with(5, 5)


if __name__ == "__main__":
    unittest.main()
