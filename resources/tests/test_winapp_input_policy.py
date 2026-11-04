"""Behavioral boundaries for non-intrusive WinApp action dispatch."""

import sys
import asyncio
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import winapp_service as service


def unavailable_pattern():
    raise RuntimeError("pattern unavailable")


class InputPolicyTests(unittest.TestCase):
    def setUp(self):
        # Native COM policy is tested separately; these tests isolate dispatch.
        policy = patch.object(service, "ensure_nonintrusive_uia")
        policy.start()
        self.addCleanup(policy.stop)

    def test_uia_focus_policy_failure_blocks_before_resolution_or_fallback(self):
        for action in ("click", "fill", "select"):
            session = self.session(SimpleNamespace())
            with patch.object(service, "ensure_nonintrusive_uia", side_effect=service.RequiresInteractiveInput("unsupported client")), patch.object(
                session, "_background_click"
            ) as messages, patch.object(session, "_activate_foreground") as activate:
                result = session.execute(dict(action=action, selector="uia:name=Target", value="new"))
            self.assertEqual(result["code"], "requires_interactive_input", result)
            session.find_control.assert_not_called()
            messages.assert_not_called()
            activate.assert_not_called()

    def test_navigation_select_precedes_legacy_default_action(self):
        legacy = Mock()
        selection = SimpleNamespace(Select=Mock(return_value=True), IsSelected=True)
        control = SimpleNamespace(GetSelectionItemPattern=lambda: selection,
                                  GetLegacyIAccessiblePattern=legacy)
        result = self.session(control).execute(dict(action="click", selector="uia:name=Settings"))
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["input_method"], "SelectionItemPattern.Select")
        legacy.assert_not_called()

    def test_authorized_activation_uses_observed_foreground_before_injecting(self):
        for foregrounds, activation_result, expected, activate_calls in (
            ([123], False, True, 0),
            ([0, 123], False, True, 1),
            ([0, 456], True, False, 1),
        ):
            session = self.session()
            session.find_visible_interactive_control = Mock(return_value=(object(),
                dict(left=0, top=0, width=10, height=10)))
            win32 = self.user32_mock(GetForegroundWindow=Mock(side_effect=foregrounds),
                                    SetForegroundWindow=Mock(return_value=activation_result))
            with patch.object(service, "user32", win32), patch.object(service, "send_pointer_click") as pointer:
                result = session.execute(dict(action="pointer_click", selector="uia:name=Close",
                    mode="foreground", allow_interactive_input=True))
            self.assertEqual(result["ok"], expected, result)
            self.assertEqual(pointer.call_count, int(expected))
            self.assertEqual(win32.SetForegroundWindow.call_count, activate_calls)

    def test_selection_readback_failure_never_attempts_another_action(self):
        legacy = Mock()
        control = SimpleNamespace(GetSelectionItemPattern=lambda: SimpleNamespace(
            Select=Mock(return_value=True), IsSelected=False), GetLegacyIAccessiblePattern=legacy)
        result = self.session(control).execute(dict(action="click", selector="uia:name=Settings"))
        self.assertEqual(result["code"], "action_result_uncertain", result)
        legacy.assert_not_called()

    def test_permission_matrix_direct_and_sidecar_never_injects_without_consent(self):
        for mode in ("auto", "background", "foreground"):
            for permission in (None, False, "true", 1, True):
                if mode == "foreground" and permission is True:
                    continue
                for cmd in ("execute_locator", "shell_tray_click"):
                    with self.subTest(mode=mode, permission=permission, cmd=cmd):
                        session = self.session()
                        payload = dict(cmd=cmd, action="pointer_click", selector="uia:name=Close",
                                       name="Fixture", mode=mode, allow_interactive_input=permission)
                        with patch.object(service, "send_pointer_click") as inject, patch.object(
                            service, "reveal_autohidden_taskbar"
                        ) as reveal, patch.object(session, "_activate_foreground") as activate:
                            result = asyncio.run(service.handle_command(session, payload))
                        self.assertEqual(result["code"], "requires_interactive_input", result)
                        inject.assert_not_called()
                        reveal.assert_not_called()
                        activate.assert_not_called()

    def test_uia_failure_is_uncertain_and_never_retried(self):
        invoke = Mock(side_effect=RuntimeError("provider disconnected after dispatch"))
        control = SimpleNamespace(GetInvokePattern=lambda: SimpleNamespace(Invoke=invoke))
        session = self.session(control)
        with patch.object(session, "_background_click") as background:
            result = session.execute(dict(action="click", selector="uia:name=Save"))
        self.assertEqual(result["code"], "action_result_uncertain", result)
        background.assert_not_called()

    def test_unknown_uia_completion_never_becomes_business_success(self):
        for completion in (False, None):
            control = SimpleNamespace(GetInvokePattern=lambda: SimpleNamespace(Invoke=lambda: completion))
            result = self.session(control).execute(dict(action="click", selector="uia:name=Save"))
            self.assertEqual(result["code"], "action_result_uncertain", result)

    def test_failed_authorized_focus_never_sends_keys(self):
        control = SimpleNamespace(SetFocus=Mock(return_value=False), GetValuePattern=unavailable_pattern)
        session = self.session(control)
        fake_auto = SimpleNamespace(SendKeys=Mock())
        with patch.object(service, "user32", self.user32_mock()), patch.object(service, "auto", fake_auto):
            result = session.execute(dict(action="fill", selector="uia:name=Edit", value="new",
                                          mode="foreground", allow_interactive_input=True))
        self.assertEqual(result["code"], "action_result_uncertain", result)
        fake_auto.SendKeys.assert_not_called()

    def test_fill_transport_success_does_not_prove_value_changed(self):
        control = SimpleNamespace(GetValuePattern=unavailable_pattern, NativeWindowHandle=456, Name="Old")
        session = self.session(control)
        with patch.object(service, "user32", self.user32_mock()):
            result = session.execute(dict(action="fill", selector="uia:name=Edit", value="New"))
        self.assertEqual(result["code"], "action_result_uncertain", result)

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
            "IsWindowVisible": Mock(return_value=True),
            "IsIconic": Mock(return_value=False),
            "SetForegroundWindow": Mock(return_value=True),
            "GetForegroundWindow": Mock(side_effect=[0, 123]),
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
            IsOffscreen=False,
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
                    "allow_interactive_input": True,
                }
            )
            session.control_exists = Mock(return_value=False)
            absent = session.execute(
                {
                    "selector": "uia:name=Missing",
                    "action": "assert_not_exists",
                    "mode": "foreground",
                    "allow_interactive_input": True,
                }
            )

        self.assertTrue(visible["ok"], visible)
        self.assertTrue(text["ok"], text)
        self.assertTrue(absent["ok"], absent)
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_click_invoke_success_never_activates_foreground(self):
        invoke = Mock(return_value=True)
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
                    "allow_interactive_input": True,
                }
            )

        self.assertTrue(result["ok"], result)
        invoke.assert_called_once_with()
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_fill_value_pattern_success_never_focuses_or_activates(self):
        set_value = Mock(return_value=True)
        control = SimpleNamespace(
            GetValuePattern=lambda: SimpleNamespace(SetValue=set_value, Value="Ada"),
            SetFocus=Mock(return_value=True),
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
                    "allow_interactive_input": True,
                }
            )

        self.assertTrue(result["ok"], result)
        set_value.assert_called_once_with("Ada")
        control.SetFocus.assert_not_called()
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_fill_without_control_hwnd_uses_explicit_foreground_fallback(self):
        control = SimpleNamespace(
            GetValuePattern=unavailable_pattern,
            SetFocus=Mock(return_value=True),
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
                    "allow_interactive_input": True,
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
        self.assertEqual(result["code"], "requires_interactive_input")
        self.assertIn("no control HWND for background fill", result["detail"])
        fake_user32.SendMessageTimeoutW.assert_not_called()
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_select_pattern_success_never_activates_foreground(self):
        select = Mock(return_value=True)
        control = SimpleNamespace(
            GetSelectionItemPattern=lambda: SimpleNamespace(Select=select, IsSelected=True),
        )
        fake_user32 = self.user32_mock()
        session = self.session(control)

        with patch.object(service, "user32", fake_user32):
            result = session.execute(
                {
                    "selector": "uia:name=Choice",
                    "action": "select",
                    "mode": "foreground",
                    "allow_interactive_input": True,
                }
            )

        self.assertTrue(result["ok"], result)
        select.assert_called_once_with()
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_pointer_click_requires_explicit_permission_and_uses_real_pointer(self):
        fake_user32 = self.user32_mock()
        session = self.session()
        session.find_visible_interactive_control = Mock(
            return_value=(object(), {"left": 10, "top": 20, "width": 20, "height": 10})
        )

        with patch.object(service, "user32", fake_user32), patch.object(
            service, "send_pointer_click"
        ) as inject:
            result = session.execute(
                {"selector": "uia:name=Close", "action": "pointer_click", "mode": "foreground", "allow_interactive_input": True}
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

        self.assertFalse(result["ok"], result)
        self.assertEqual(result["code"], "action_result_uncertain")
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

    def test_partial_background_press_key_never_retries_foreground(self):
        control = SimpleNamespace(NativeWindowHandle=456, SetFocus=Mock(return_value=True))
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
                    "allow_interactive_input": True,
                }
            )

        self.assertTrue(result, result)
        self.assertEqual(result["code"], "action_result_uncertain")
        fake_user32.SetForegroundWindow.assert_not_called()
        control.SetFocus.assert_not_called()
        fake_auto.SendKeys.assert_not_called()

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
        self.assertEqual(result["code"], "requires_interactive_input")
        fake_user32.SetForegroundWindow.assert_not_called()

    def test_replay_style_sequence_activates_only_pointer_step(self):
        control = SimpleNamespace(BoundingRectangle=(0, 0, 10, 10), IsOffscreen=False)
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
                {"selector": "uia:name=Close", "action": "pointer_click", "mode": "foreground", "allow_interactive_input": True}
            )

        self.assertTrue(assertion["ok"], assertion)
        self.assertTrue(pointer["ok"], pointer)
        fake_user32.SetForegroundWindow.assert_called_once_with(123)
        inject.assert_called_once_with(5, 5)


class UiaFocusClientTests(unittest.TestCase):
    def test_replaces_the_actual_shared_legacy_client_and_walker(self):
        legacy = SimpleNamespace(QueryInterface=Mock(side_effect=RuntimeError("E_NOINTERFACE")))
        safe = SimpleNamespace(AutoSetFocus=True, RawViewWalker=object())
        safe.QueryInterface = Mock(return_value=safe)
        shared = SimpleNamespace(IUIAutomation=legacy, ViewWalker=object(),
                                 UIAutomationCore=SimpleNamespace(IUIAutomation2=object()))
        backend = SimpleNamespace(_AutomationClient=SimpleNamespace(instance=lambda: shared))
        create = Mock(return_value=safe)
        actual = service.configure_nonintrusive_uia_client(backend, create)
        self.assertIs(actual, safe)
        self.assertIs(shared.IUIAutomation, safe)
        self.assertIs(shared.ViewWalker, safe.RawViewWalker)
        self.assertIs(safe.AutoSetFocus, False)
        create.assert_called_once_with("{e22ad333-b25f-460c-83d0-0581107395c9}",
                                       interface=shared.UIAutomationCore.IUIAutomation2)
        service.configure_nonintrusive_uia_client(backend, create)
        self.assertEqual(create.call_count, 1)

    def test_rejects_policy_readback_failure_before_publishing_client(self):
        class UnsafeClient:
            RawViewWalker = object()
            @property
            def AutoSetFocus(self):
                return True
            @AutoSetFocus.setter
            def AutoSetFocus(self, value):
                pass
        legacy = SimpleNamespace(QueryInterface=Mock(side_effect=RuntimeError("unsupported")))
        shared = SimpleNamespace(IUIAutomation=legacy, ViewWalker=object(),
                                 UIAutomationCore=SimpleNamespace(IUIAutomation2=object()))
        backend = SimpleNamespace(_AutomationClient=SimpleNamespace(instance=lambda: shared))
        with self.assertRaisesRegex(RuntimeError, "remained enabled"):
            service.configure_nonintrusive_uia_client(backend, Mock(return_value=UnsafeClient()))
        self.assertIs(shared.IUIAutomation, legacy)


if __name__ == "__main__":
    unittest.main()
