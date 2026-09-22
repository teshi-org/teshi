"""Regression tests for the WinApp integrity boundary."""

from __future__ import annotations

import asyncio
import os
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

RESOURCES = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RESOURCES))

import winapp_service as service  # noqa: E402


class WinAppIntegrityTests(unittest.TestCase):
    def test_integrity_levels_require_exact_known_match(self) -> None:
        self.assertIsNone(
            service.integrity_mismatch_message(
                service.IntegrityLevel.MEDIUM,
                service.IntegrityLevel.MEDIUM,
            )
        )
        for executor, target in (
            (service.IntegrityLevel.LOW, service.IntegrityLevel.MEDIUM),
            (service.IntegrityLevel.HIGH, service.IntegrityLevel.SYSTEM),
            (service.IntegrityLevel.UNKNOWN, service.IntegrityLevel.MEDIUM),
            (service.IntegrityLevel.MEDIUM, service.IntegrityLevel.UNKNOWN),
        ):
            with self.subTest(executor=executor, target=target):
                self.assertIsNotNone(service.integrity_mismatch_message(executor, target))

    def test_unknown_target_integrity_fails_closed_before_click(self) -> None:
        session = service.WinAppSession(None)
        with patch.object(
            session,
            "integrity_status",
            return_value={
                "executor_pid": 10,
                "executor_integrity": "Medium",
                "target_pid": 20,
                "target_integrity": "Unknown",
                "target_elevated": False,
                "integrity_match": False,
            },
        ), patch.object(session, "find_control") as find_control:
            response = session.execute(
                {"selector": "uia:name=User", "action": "click"}
            )

        self.assertFalse(response["ok"])
        self.assertEqual(response["code"], "integrity_mismatch")
        self.assertIn("Unable to verify", response["error"])
        find_control.assert_not_called()

    def test_exec_actions_pass_through_integrity_guard(self) -> None:
        session = service.WinAppSession(None)
        error = {"ok": False, "code": "integrity_mismatch", "error": "blocked"}
        with (
            patch.object(session, "integrity_guard", return_value=error) as guard,
            patch.object(session, "_handle_exec") as handle_exec,
        ):
            response = session.execute(
                {"selector": "launch", "action": "exec", "value": "app.exe"}
            )

        self.assertEqual(response, {**error, "selector": "launch"})
        guard.assert_called_once_with("exec")
        handle_exec.assert_not_called()

    def test_close_propagates_post_message_failure(self) -> None:
        session = service.WinAppSession(None)
        session.hwnd = 123
        with (
            patch.object(session, "integrity_guard", return_value=None),
            patch.object(
                service,
                "user32",
                SimpleNamespace(PostMessageW=lambda *_args: False),
            ),
        ):
            response = session.execute(
                {"selector": "close", "action": "exec", "value": ""}
            )

        self.assertFalse(response["ok"])
        self.assertIn("close failed", response["error"])

    def test_screenshot_guard_runs_before_capture(self) -> None:
        session = service.WinAppSession(None)
        error = {"ok": False, "code": "integrity_mismatch", "error": "blocked"}

        async def run() -> dict:
            with (
                patch.object(session, "integrity_guard", return_value=error) as guard,
                patch.object(session, "capture_jpeg") as capture,
            ):
                response = await service.handle_command(
                    session,
                    {"cmd": "screenshot", "request_id": "shot"},
                )
            guard.assert_called_once_with("screenshot")
            capture.assert_not_called()
            return response

        response = asyncio.run(run())
        self.assertEqual(response, error)

    def test_preview_guard_blocks_frame_capture(self) -> None:
        session = service.WinAppSession(None)
        session.hwnd = 123
        sent: list[str] = []

        class Client:
            async def send(self, message: str) -> None:
                sent.append(message)

        error = {"ok": False, "code": "integrity_mismatch", "error": "blocked"}
        session.clients.add(Client())

        async def run() -> None:
            with (
                patch.object(session, "integrity_guard", return_value=error) as guard,
                patch.object(session, "capture_jpeg") as capture,
            ):
                task = asyncio.create_task(session.broadcast_frame_loop())
                await asyncio.sleep(service.FRAME_INTERVAL_SEC * 2)
                task.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await task
            guard.assert_called()
            capture.assert_not_called()

        asyncio.run(run())
        self.assertTrue(sent)
        self.assertIn('"type": "frame_error"', sent[0])

    def test_websocket_auth_requires_one_matching_query_token(self) -> None:
        self.assertEqual(
            service.authenticated_websocket_path("/?token=secret", "secret"),
            "/",
        )
        for path in ("/", "/?token=wrong", "/?token=secret&token=secret"):
            with self.subTest(path=path):
                self.assertIsNone(service.authenticated_websocket_path(path, "secret"))

    def test_integrity_rid_parser_distinguishes_medium_and_high(self) -> None:
        self.assertEqual(service.parse_integrity_level(0x2000), service.IntegrityLevel.MEDIUM)
        self.assertEqual(service.parse_integrity_level(0x2100), service.IntegrityLevel.MEDIUM)
        self.assertEqual(service.parse_integrity_level(0x3000), service.IntegrityLevel.HIGH)

    @unittest.skipUnless(os.name == "nt", "Windows token API is required")
    def test_current_ordinary_process_is_medium(self) -> None:
        self.assertEqual(
            service.current_process_integrity(),
            service.IntegrityLevel.MEDIUM,
        )

    def test_medium_executor_rejects_high_target_before_click(self) -> None:
        session = service.WinAppSession(None)
        with patch.object(
            session,
            "integrity_status",
            return_value={
                "executor_pid": 10,
                "executor_integrity": "Medium",
                "target_pid": 20,
                "target_integrity": "High",
                "target_elevated": True,
                "integrity_match": False,
            },
        ), patch.object(session, "find_control") as find_control:
            response = session.execute(
                {"selector": "uia:name=Administrator", "action": "click"}
            )

        self.assertFalse(response["ok"])
        self.assertEqual(response["code"], "integrity_mismatch")
        self.assertIn("executor is not elevated", response["error"])
        find_control.assert_not_called()

    def test_medium_executor_can_run_medium_target_click(self) -> None:
        session = service.WinAppSession(None)
        control = object()
        with (
            patch.object(
                session,
                "integrity_status",
                return_value={
                    "executor_pid": 10,
                    "executor_integrity": "Medium",
                    "target_pid": 20,
                    "target_integrity": "Medium",
                    "target_elevated": False,
                    "integrity_match": True,
                },
            ),
            patch.object(session, "find_control", return_value=control),
            patch.object(session, "_click") as click,
        ):
            response = session.execute({"selector": "uia:name=User", "action": "click"})

        self.assertTrue(response["ok"])
        click.assert_called_once_with(control, "auto")

    def test_status_without_target_reports_executor_only(self) -> None:
        session = service.WinAppSession(None)
        with patch.object(
            service,
            "current_process_integrity",
            return_value=service.IntegrityLevel.MEDIUM,
        ):
            response = session.status()

        self.assertTrue(response["ok"])
        self.assertEqual(response["executor_integrity"], "Medium")
        self.assertIsNone(response["target_pid"])
        self.assertFalse(response["target_elevated"])
        self.assertFalse(response["target"]["attached"])


if __name__ == "__main__":
    unittest.main()
