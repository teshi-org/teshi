"""Lossless UIA capture contracts without a real window."""
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import winapp_service as service
from PIL import Image, PngImagePlugin


class VisualTests(unittest.TestCase):
    def test_crop_physical_negative_screen_origin_and_exact_pixels(self):
        image = Image.new("RGB", (10, 8))
        image.putpixel((3, 2), (21, 42, 63))
        crop = service.crop_element(image, (-100, 200, -90, 208),
                                    dict(x=-97, y=202, width=4, height=3))
        self.assertEqual(crop.size, (4, 3))
        self.assertEqual(crop.getpixel((0, 0)), (21, 42, 63))

    def test_crop_rejects_all_edges_empty_fractional_and_frame_size_mismatch(self):
        image = Image.new("RGB", (10, 8))
        for bounds in [(-1, 0, 1, 1), (0, -1, 1, 1), (9, 0, 2, 1),
                       (0, 7, 1, 2), (0, 0, 0, 1), (0.5, 0, 1, 1)]:
            with self.subTest(bounds=bounds), self.assertRaises(RuntimeError):
                service.crop_element(image, (0, 0, 10, 8), dict(zip(("x", "y", "width", "height"), bounds)))
        with self.assertRaises(RuntimeError):
            service.crop_element(image, (0, 0, 20, 16), dict(x=0, y=0, width=1, height=1))

    def test_selector_excludes_offscreen_and_rejects_ambiguity(self):
        visible = dict(name="Close", control_type="ButtonControl", is_offscreen=False)
        hidden = {**visible, "is_offscreen": True}
        snapshot = dict(interactive_elements=[hidden, visible])
        selector = "uia:control_type=ButtonControl;name=Close"
        self.assertIs(service.resolve_screenshot_element(snapshot, selector), visible)
        for nodes in [[hidden], [visible, visible]]:
            with self.assertRaises(RuntimeError):
                service.resolve_screenshot_element(dict(interactive_elements=nodes), selector)
        with self.assertRaises(RuntimeError):
            service.resolve_screenshot_element({**snapshot, "truncated": True}, selector)

    def test_pointer_coordinates_support_negative_virtual_desktop_origin(self):
        metrics = {
            service.SM_XVIRTUALSCREEN: -100,
            service.SM_YVIRTUALSCREEN: -50,
            service.SM_CXVIRTUALSCREEN: 300,
            service.SM_CYVIRTUALSCREEN: 200,
        }
        send_input = Mock(return_value=3)
        fake_user32 = SimpleNamespace(
            GetSystemMetrics=lambda metric: metrics[metric],
            SendInput=send_input,
        )
        with patch.object(service, "user32", fake_user32):
            service.send_pointer_click(-50, 25)

        inputs = send_input.call_args.args[1]
        self.assertEqual(inputs[0].mi.dwFlags,
                         service.MOUSEEVENTF_MOVE
                         | service.MOUSEEVENTF_ABSOLUTE
                         | service.MOUSEEVENTF_VIRTUALDESK)
        self.assertEqual(inputs[0].mi.dx, round(50 * 65535 / 299))
        self.assertEqual(inputs[0].mi.dy, round(75 * 65535 / 199))
        self.assertEqual(inputs[1].mi.dwFlags, service.MOUSEEVENTF_LEFTDOWN)
        self.assertEqual(inputs[2].mi.dwFlags, service.MOUSEEVENTF_LEFTUP)

    def test_send_input_failure_is_not_reported_as_success(self):
        metrics = {
            service.SM_XVIRTUALSCREEN: 0,
            service.SM_YVIRTUALSCREEN: 0,
            service.SM_CXVIRTUALSCREEN: 1920,
            service.SM_CYVIRTUALSCREEN: 1080,
        }
        fake_user32 = SimpleNamespace(
            GetSystemMetrics=lambda metric: metrics[metric],
            SendInput=Mock(return_value=0),
        )
        with patch.object(service, "user32", fake_user32), self.assertRaisesRegex(
            RuntimeError, "SendInput failed"
        ):
            service.send_pointer_click(100, 200)

    def test_pointer_resolver_ignores_offscreen_and_rejects_ambiguity_or_bad_bounds(self):
        hidden_control = SimpleNamespace(
            Name="Close",
            ControlTypeName="ButtonControl",
            IsOffscreen=True,
            BoundingRectangle=(0, 0, 10, 10),
            GetChildren=lambda: [],
        )
        visible_control = SimpleNamespace(
            Name="Close",
            ControlTypeName="ButtonControl",
            IsOffscreen=False,
            BoundingRectangle=(10, 20, 30, 30),
            GetChildren=lambda: [],
        )
        root = SimpleNamespace(
            GetChildren=lambda: [hidden_control, visible_control]
        )
        session = service.WinAppSession(None)
        session.root_control = Mock(return_value=root)
        selector = "uia:control_type=ButtonControl;name=Close"
        hidden = dict(
            path="0/0", name="Close", control_type="ButtonControl",
            is_offscreen=True,
            bounding_rectangle=dict(left=0, top=0, width=10, height=10),
        )
        visible = dict(
            path="0/1", name="Close", control_type="ButtonControl",
            is_offscreen=False,
            bounding_rectangle=dict(left=10, top=20, width=20, height=10),
        )
        session.snapshot = Mock(return_value={"interactive_elements": [hidden, visible]})
        control, bounds = session.find_visible_interactive_control(selector)
        self.assertIs(control, visible_control)
        self.assertEqual(bounds["width"], 20)

        session.snapshot.return_value = {
            "interactive_elements": [{**visible, "path": "0/0"}]
        }
        with self.assertRaisesRegex(RuntimeError, "became offscreen"):
            session.find_visible_interactive_control(selector)

        session.snapshot.return_value = {
            "interactive_elements": [visible, visible]
        }
        with self.assertRaisesRegex(RuntimeError, "2 visible interactive"):
            session.find_visible_interactive_control(selector)

        session.snapshot.return_value = {
            "truncated": True,
            "interactive_elements": [visible],
        }
        with self.assertRaisesRegex(RuntimeError, "snapshot is incomplete"):
            session.find_visible_interactive_control(selector)

        session.snapshot.return_value = {
            "interactive_elements": [{**visible, "bounding_rectangle": None}]
        }
        with self.assertRaisesRegex(RuntimeError, "no visible UIA screen bounds"):
            session.find_visible_interactive_control(selector)

    def test_pointer_click_requires_foreground_mode(self):
        session = service.WinAppSession(None)
        result = session.execute(
            dict(selector="uia:name=Close", action="pointer_click", mode="background")
        )
        self.assertFalse(result["ok"])
        self.assertIn("pointer_click requires foreground mode", result["error"])

    def test_assert_not_exists_passes_only_when_uia_selector_is_absent(self):
        session = service.WinAppSession(None)
        session.root_control = Mock(
            return_value=SimpleNamespace(
                Name="root",
                GetChildren=lambda: [],
            )
        )
        absent = session.execute(
            dict(selector="uia:name=Missing", action="assert_not_exists")
        )
        self.assertTrue(absent["ok"], absent)
        self.assertFalse(absent["exists"])

        session.find_control = Mock(return_value=SimpleNamespace(IsOffscreen=True))
        present = session.execute(
            dict(selector="uia:name=Hidden", action="assert_not_exists")
        )
        self.assertFalse(present["ok"], present)
        self.assertTrue(present["exists"])
        self.assertIn("element exists", present["error"])

    def test_assert_not_exists_fails_closed_when_uia_provider_is_unavailable(self):
        session = service.WinAppSession(None)
        provider_error = RuntimeError("provider unavailable")
        session.root_control = Mock(
            return_value=SimpleNamespace(
                Name="root",
                GetChildren=Mock(side_effect=provider_error),
            )
        )

        result = session.execute(
            dict(selector="uia:name=Missing", action="assert_not_exists")
        )

        self.assertFalse(result["ok"], result)
        self.assertIn("UIA provider unavailable", result["error"])
        self.assertIn("provider unavailable", result["error"])

    def test_pointer_click_moves_and_clicks_without_invoke_or_legacy_click(self):
        invoke = Mock(side_effect=AssertionError("InvokePattern must not run"))
        legacy_click = Mock(side_effect=AssertionError("legacy click must not run"))
        control = SimpleNamespace(
            Name="Close",
            ControlTypeName="ButtonControl",
            IsOffscreen=False,
            BoundingRectangle=(10, 20, 30, 40),
            GetChildren=lambda: [],
            GetInvokePattern=invoke,
            Click=legacy_click,
        )
        session = service.WinAppSession(None)
        session.hwnd = 123
        session.root_control = Mock(
            return_value=SimpleNamespace(GetChildren=lambda: [control])
        )
        session.snapshot = Mock(
            return_value={
                "interactive_elements": [
                    dict(
                        path="0/0",
                        name="Close",
                        control_type="ButtonControl",
                        is_offscreen=False,
                        bounding_rectangle=dict(
                            left=10, top=20, width=20, height=20
                        ),
                    )
                ]
            }
        )
        fake_user32 = SimpleNamespace(
            SetForegroundWindow=Mock(return_value=True),
            SetThreadDpiAwarenessContext=Mock(return_value=1),
        )
        with patch.object(service, "user32", fake_user32), patch.object(
            service, "send_pointer_click"
        ) as inject:
            result = session.execute(
                dict(
                    selector="uia:control_type=ButtonControl;name=Close",
                    action="pointer_click",
                )
            )

        self.assertTrue(result["ok"], result)
        inject.assert_called_once_with(20, 30)
        invoke.assert_not_called()
        legacy_click.assert_not_called()

    def test_click_keeps_invoke_pattern_first(self):
        invoke = Mock()
        legacy_click = Mock()
        control = SimpleNamespace(
            GetInvokePattern=lambda: SimpleNamespace(Invoke=invoke),
            Click=legacy_click,
        )
        session = service.WinAppSession(None)
        session.find_control = Mock(return_value=control)

        result = session.execute(
            dict(selector="uia:name=Close", action="click")
        )

        self.assertTrue(result["ok"], result)
        invoke.assert_called_once_with()
        legacy_click.assert_not_called()

    def test_rgb_tolerance_counts_pixels_not_channels(self):
        baseline = Image.new("RGB", (3, 1), (100, 100, 100))
        self.assertEqual(service.compare_pixels(baseline, baseline, 0)[0], 0)
        actual = baseline.copy()
        actual.putpixel((0, 0), (92, 108, 100))
        self.assertEqual(service.compare_pixels(baseline, actual, 8)[0], 0)
        actual.putpixel((1, 0), (109, 109, 109))
        actual.putpixel((2, 0), (100, 91, 100))
        count, diff = service.compare_pixels(baseline, actual, 8)
        self.assertEqual(count, 2)
        self.assertEqual(diff.getpixel((1, 0)), (255, 0, 255))

    def test_dimensions_mismatch_counts_missing_pixels(self):
        count, diff = service.compare_pixels(Image.new("RGB", (2, 2)), Image.new("RGB", (3, 2)), 8)
        self.assertEqual(count, 2)
        self.assertEqual(diff.size, (3, 2))

    def test_json_capture_assertion_diff_and_recorded_bounds(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            session = service.WinAppSession(root)
            session.hwnd = 123
            node = dict(name="Close", is_offscreen=False, bounding_rectangle=dict(left=11, top=22, width=3, height=2))
            session.snapshot = Mock(return_value=dict(interactive_elements=[node]))
            backend = Mock()
            backend.capture_rgb.return_value = (Image.new("RGB", (10, 10), (100, 100, 100)), (10, 20, 20, 30))
            session._capture_backend = backend
            with patch.object(service, "user32", SimpleNamespace(GetDpiForWindow=lambda hwnd: 144, SetThreadDpiAwarenessContext=lambda context: 1)):
                saved = session.visual_command(dict(cmd="element_screenshot", selector="uia:name=Close", out="base.png"))
                self.assertTrue(saved["ok"], saved)
                self.assertEqual(saved["dpi"], 144)
                self.assertEqual(saved["bounds"], dict(x=11, y=22, width=3, height=2))
                command = dict(selector="uia:name=Close", baseline="base.png", diff_out="diff.png")
                same = session.visual_command(command)
                self.assertTrue(same["ok"], same)
                self.assertFalse((root / "diff.png").exists())
                backend.capture_rgb.return_value[0].putpixel((1, 2), (109, 100, 100))
                different = session.visual_command(command)
                self.assertFalse(different["ok"])
                self.assertEqual(different["changed_pixels"], 1)
                self.assertEqual(different["actual_dimensions"], different["baseline_dimensions"])
                with Image.open(root / "diff.png") as diff:
                    self.assertEqual(diff.format, "PNG")
                backend.capture_rgb.return_value[0].putpixel((1, 2), (100, 100, 100))
                node["bounding_rectangle"]["width"] = 4
                resized = session.visual_command(command)
                self.assertFalse(resized["ok"])
                self.assertEqual(resized["bounds"]["width"], 4)
                self.assertEqual(resized["actual_dimensions"]["width"], 4)
                self.assertEqual(resized["baseline_dimensions"]["width"], 3)
                self.assertEqual(resized["error"], "element dimensions changed")
                node["bounding_rectangle"]["width"] = 3
                metadata = PngImagePlugin.PngInfo()
                metadata.add_text("teshi_bounds", '{"width":4,"height":2}')
                Image.new("RGB", (3, 2), (100, 100, 100)).save(root / "base.png", pnginfo=metadata)
                result = session.visual_command(command)
                self.assertFalse(result["ok"])
                self.assertEqual(result["error"], "element dimensions changed")
                self.assertEqual(result["changed_pixels"], 0)
                Image.new("RGB", (4, 2)).save(root / "base.png")
                self.assertFalse(session.visual_command(command)["ok"])
                backend.capture_jpeg.assert_not_called()

    def test_snapshot_screenshot_sees_ambiguity_beyond_preview_limit(self):
        def button(name):
            return SimpleNamespace(Name=name, ControlTypeName="ButtonControl", IsOffscreen=False,
                                   BoundingRectangle=(0, 0, 2, 2), GetChildren=lambda: [])
        controls = [button("Close"), *[button(str(i)) for i in range(80)], button("Close")]
        session = service.WinAppSession(None)
        session.root_control = Mock(return_value=SimpleNamespace(Name="root", IsOffscreen=False,
                                                               GetChildren=lambda: controls))
        self.assertEqual(len(session.snapshot()["interactive_elements"]), 80)
        with self.assertRaisesRegex(RuntimeError, "2 visible interactive"):
            service.resolve_screenshot_element(session.snapshot(for_screenshot=True), "uia:name=Close")

    def test_assert_execute_avoids_general_find_control(self):
        session = service.WinAppSession(None)
        session.visual_command = Mock(return_value={"ok": False, "changed_pixels": 3})
        session.find_control = Mock(side_effect=AssertionError("legacy resolver called"))
        result = session.execute(dict(selector="uia:name=Close", action="assert_screenshot", value="base.png"))
        self.assertFalse(result["ok"])
        self.assertEqual(session.visual_command.call_args.args[0]["baseline"], "base.png")

    def test_success_removes_previous_failure_diff(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            Image.new("RGB", (2, 2)).save(root / "base.png")
            Image.new("RGB", (2, 2), "magenta").save(root / "diff.png")
            session = service.WinAppSession(root)
            session.hwnd = 1
            session.snapshot = Mock(return_value={"interactive_elements": [dict(name="Close", is_offscreen=False,
                bounding_rectangle=dict(left=0, top=0, width=2, height=2))]})
            session._capture_backend = Mock()
            session._capture_backend.capture_rgb.return_value = (Image.new("RGB", (2, 2)), (0, 0, 2, 2))
            with patch.object(service, "user32", SimpleNamespace(GetDpiForWindow=lambda hwnd: 96, SetThreadDpiAwarenessContext=lambda c: 1)):
                result = session.visual_command(dict(selector="uia:name=Close", baseline="base.png", diff_out="diff.png"))
                self.assertTrue(result["ok"], result)
                self.assertFalse((root / "diff.png").exists())

    def test_imagegrab_lossless_path_uses_screen_capture_without_cursor_compositing(self):
        source = Image.new("RGB", (4, 4), (3, 7, 11))
        with patch.object(service, "get_window_rect", return_value=(-4, 10, 0, 14)), patch.object(service, "ImageGrab") as grab:
            grab.grab.return_value = source
            result, bounds = service.ImageGrabCaptureBackend(1).capture_rgb()
            grab.grab.assert_called_once_with(bbox=(-4, 10, 0, 14), all_screens=True)
            self.assertEqual(result.tobytes(), source.tobytes())
            self.assertEqual(bounds, (-4, 10, 0, 14))

    def test_wgc_retains_uncompressed_pixels_and_refreshes_static_frame(self):
        from test_winapp_capture_backend import FakeCapture

        source = Image.new("RGB", (2, 2), (1, 9, 17))
        class AutoCapture(FakeCapture):
            def start_free_threaded(self):
                # Deliver an initial frame after event registration, as WGC does.
                self.emit(b"raw-bgra")
                return self.control

        with patch.object(service, "WindowsCapture", AutoCapture), patch.object(service, "get_capture_bounds", return_value=(0, 0, 2, 2)), \
             patch.object(service.Image, "fromarray", return_value=source):
            backend = service.WgcCaptureBackend(1)
            old = backend._control
            captured, bounds = backend.capture_rgb()
            self.assertTrue(old.stopped)
            self.assertFalse(backend._capture.settings["cursor_capture"])
            self.assertEqual(captured.tobytes(), source.tobytes())
            self.assertEqual(bounds, (0, 0, 2, 2))
            self.assertIsNot(captured, source)
            backend.stop()

    def test_legacy_find_control_still_selects_first_matching_offscreen_node(self):
        hidden = SimpleNamespace(Name="Close", ControlTypeName="ButtonControl", IsOffscreen=True, GetChildren=lambda: [])
        visible = SimpleNamespace(Name="Close", ControlTypeName="ButtonControl", IsOffscreen=False, GetChildren=lambda: [])
        session = service.WinAppSession(None)
        session.root_control = Mock(return_value=SimpleNamespace(Name="root", GetChildren=lambda: [hidden, visible]))
        self.assertIs(session.find_control("uia:name=Close"), hidden)
        session._click = Mock()
        self.assertTrue(session.execute(dict(selector="uia:name=Close", action="click"))["ok"])
        session._click.assert_called_once_with(hidden)


if __name__ == "__main__":
    unittest.main()
