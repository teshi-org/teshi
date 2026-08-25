"""Regression tests for the WinApp WGC/ImageGrab capture backend boundary."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

RESOURCES = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RESOURCES))

import winapp_service as service  # noqa: E402


class FakeControl:
    def __init__(self) -> None:
        self.stopped = False

    def stop(self) -> None:
        self.stopped = True

    def wait(self) -> None:
        return

    def is_finished(self) -> bool:
        return self.stopped


class FakeCapture:
    instances: list["FakeCapture"] = []

    def __init__(self, **settings: object) -> None:
        self.settings = settings
        self.handlers: dict[str, object] = {}
        self.control = FakeControl()
        self.instances.append(self)

    def event(self, handler: object) -> object:
        self.handlers[getattr(handler, "__name__")] = handler
        return handler

    def start_free_threaded(self) -> FakeControl:
        return self.control

    def emit(self, marker: bytes) -> None:
        frame = SimpleNamespace(frame_buffer=FakePixels(marker))
        self.handlers["on_frame_arrived"](frame, self.control)


class FakePixels:
    def __init__(self, marker: bytes) -> None:
        self.marker = marker

    def __getitem__(self, _key: object) -> bytes:
        return self.marker


class FakeEncodedImage:
    def __init__(self, marker: bytes = b"imagegrab") -> None:
        self.marker = marker

    def convert(self, _mode: str) -> "FakeEncodedImage":
        return self

    def save(self, buffer: object, **_kwargs: object) -> None:
        buffer.write(b"jpeg:" + self.marker)


class FakeImageModule:
    @staticmethod
    def fromarray(marker: bytes, mode: str) -> FakeEncodedImage:
        assert mode == "RGB"
        return FakeEncodedImage(marker)


class FakeImageGrabModule:
    @staticmethod
    def grab(**_kwargs: object) -> FakeEncodedImage:
        return FakeEncodedImage()


class WinAppCaptureBackendTests(unittest.TestCase):
    def setUp(self) -> None:
        FakeCapture.instances.clear()

    def test_wgc_uses_exact_hwnd_and_replaces_latest_frame(self) -> None:
        with patch.object(service, "Image", FakeImageModule):
            backend = service.WgcCaptureBackend(1234, capture_factory=FakeCapture)
            capture = FakeCapture.instances[-1]

            self.assertEqual(capture.settings["window_hwnd"], 1234)
            self.assertEqual(capture.settings["minimum_update_interval"], 125)
            self.assertFalse(capture.settings["cursor_capture"])
            self.assertFalse(capture.settings["draw_border"])
            self.assertFalse(capture.settings["secondary_window"])

            capture.emit(b"first")
            capture.emit(b"latest")

            self.assertEqual(backend.capture_jpeg(timeout=0), b"jpeg:latest")
            backend.stop()
            self.assertTrue(capture.control.stopped)

    def test_reattach_stops_old_wgc_session(self) -> None:
        with (
            patch.object(service, "WindowsCapture", FakeCapture),
            patch.object(service, "Image", FakeImageModule),
        ):
            session = service.WinAppSession(None)
            session._attach_target({"hwnd": 101, "title": "First"})
            first_control = FakeCapture.instances[-1].control
            session._attach_target({"hwnd": 202, "title": "Second"})

            self.assertTrue(first_control.stopped)
            self.assertEqual(session.hwnd, 202)
            self.assertEqual(session.capture_backend_name, "wgc")
            session.close()

    def test_missing_wgc_dependency_falls_back_with_reason(self) -> None:
        with (
            patch.object(service, "WindowsCapture", None),
            patch.object(service, "WGC_IMPORT_ERROR", "module not installed"),
            patch.object(service, "ImageGrab", FakeImageGrabModule),
            patch.object(service, "get_window_rect", return_value=(0, 0, 100, 100)),
        ):
            session = service.WinAppSession(None)
            session._attach_target({"hwnd": 303, "title": "Fallback"})

            self.assertEqual(session.capture_backend_name, "imagegrab")
            self.assertIn("module not installed", session._capture_fallback_reason)
            self.assertEqual(session.capture_jpeg(), b"jpeg:imagegrab")
            self.assertEqual(session.target_info()["capture_backend"], "imagegrab")

    def test_runtime_wgc_failure_falls_back_while_hwnd_is_valid(self) -> None:
        with (
            patch.object(service, "WindowsCapture", FakeCapture),
            patch.object(service, "Image", FakeImageModule),
            patch.object(service, "ImageGrab", FakeImageGrabModule),
            patch.object(service, "get_window_rect", return_value=(0, 0, 100, 100)),
            patch.object(service, "is_window", return_value=True),
        ):
            session = service.WinAppSession(None)
            session._attach_target({"hwnd": 404, "title": "Runtime failure"})
            backend = session._capture_backend
            with backend._lock:
                backend._terminal_error = "capture device lost"
            backend._first_frame.set()

            self.assertEqual(session.capture_jpeg(), b"jpeg:imagegrab")
            self.assertEqual(session.capture_backend_name, "imagegrab")
            self.assertEqual(session._capture_fallback_reason, "capture device lost")

    def test_closed_hwnd_does_not_fall_back(self) -> None:
        with (
            patch.object(service, "WindowsCapture", FakeCapture),
            patch.object(service, "Image", FakeImageModule),
            patch.object(service, "is_window", return_value=False),
        ):
            session = service.WinAppSession(None)
            session._attach_target({"hwnd": 505, "title": "Closed"})
            backend = session._capture_backend
            with backend._lock:
                backend._terminal_error = "WGC capture session closed"
            backend._first_frame.set()

            with self.assertRaisesRegex(RuntimeError, "target window closed"):
                session.capture_jpeg()
            self.assertEqual(session.capture_backend_name, "wgc")


class WinAppCaptureProtocolTests(unittest.IsolatedAsyncioTestCase):
    async def test_screenshot_reports_active_backend_metadata(self) -> None:
        with (
            patch.object(service, "WindowsCapture", None),
            patch.object(service, "WGC_IMPORT_ERROR", "not supported"),
            patch.object(service, "ImageGrab", FakeImageGrabModule),
            patch.object(service, "get_window_rect", return_value=(0, 0, 100, 100)),
        ):
            session = service.WinAppSession(None)
            session._attach_target({"hwnd": 606, "title": "Protocol"})
            response = await service.handle_command(
                session,
                {"cmd": "screenshot", "request_id": "shot"},
            )

            self.assertTrue(response["ok"])
            self.assertEqual(response["capture_backend"], "imagegrab")
            self.assertEqual(response["capture_fallback_reason"], "windows-capture is unavailable: not supported")
            session.close()


if __name__ == "__main__":
    unittest.main()
