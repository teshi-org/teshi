"""Regression tests for the browser bridge's small HTTP server."""

from __future__ import annotations

import asyncio
import json
import os
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

RESOURCES = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RESOURCES))

from browser_service import (  # noqa: E402
    BrowserSessionBroker,
    _bind_websocket_listener,
    _read_http_request,
    embedded_browser_headless,
    handle_embedded_command,
    paths_equal,
)


class FakeEmbeddedSession:
    def __init__(self) -> None:
        self.url = "about:blank"

    def current_url(self) -> str:
        return self.url

    async def navigate(self, url: str) -> None:
        self.url = url

    async def wait_for_browser_condition(
        self,
        _wait: dict[str, object] | None,
        _timeout_ms: int,
        _selector: str,
        _candidate: dict[str, object] | None,
    ) -> dict[str, object]:
        return {"ok": True}


class BrowserServiceHttpTests(unittest.IsolatedAsyncioTestCase):
    def test_embedded_browser_defaults_to_headless_and_can_be_opted_into_headed(self) -> None:
        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop("TESHI_EMBEDDED_HEADLESS", None)
            self.assertTrue(embedded_browser_headless())
        with patch.dict(os.environ, {"TESHI_EMBEDDED_HEADLESS": "0"}):
            self.assertFalse(embedded_browser_headless())

    async def test_embedded_navigation_without_target_uses_local_transport(self) -> None:
        session = FakeEmbeddedSession()
        result = await handle_embedded_command(
            session,
            {
                "cmd": "navigate",
                "request_id": "embedded-navigation",
                "url": "http://127.0.0.1:20253/?e2e=1",
            },
            BrowserSessionBroker(),
        )

        self.assertTrue(result["ok"])
        self.assertEqual(result["url"], "http://127.0.0.1:20253/?e2e=1")

    def test_windows_extended_project_path_matches_plain_path(self) -> None:
        plain = Path(r"D:\Dev\Rust\teshi\dev")
        extended = r"\\?\D:\Dev\Rust\teshi\dev"

        self.assertTrue(paths_equal(extended, plain))

    async def test_content_length_body_waits_for_all_tcp_chunks(self) -> None:
        payload = json.dumps({"snapshot": "x" * 200_000}).encode("utf-8")
        headers = (
            b"POST /v1/bridge/response HTTP/1.1\r\n"
            + f"Content-Length: {len(payload)}\r\n".encode("ascii")
            + b"Content-Type: application/json\r\n\r\n"
        )
        reader = asyncio.StreamReader()
        reader.feed_data(headers + payload[:4096])
        pending = asyncio.create_task(_read_http_request(reader))
        await asyncio.sleep(0)
        self.assertFalse(pending.done())
        reader.feed_data(payload[4096:])
        reader.feed_eof()

        request_line, request_headers, body = await pending
        self.assertEqual(request_line, "POST /v1/bridge/response HTTP/1.1")
        self.assertEqual(int(request_headers["content-length"]), len(payload))
        self.assertEqual(body, payload)
        self.assertEqual(json.loads(body)["snapshot"], "x" * 200_000)

    async def test_incomplete_content_length_is_rejected(self) -> None:
        fixture = json.loads(
            (RESOURCES / "browser_contract_fixtures.json").read_text(encoding="utf-8")
        )["migration_contracts"]["malformed_transport"]["incomplete_content_length"]
        reader = asyncio.StreamReader()
        reader.feed_data(
            (
                "POST /v1/bridge/response HTTP/1.1\r\n"
                f"Content-Length: {fixture['declared_bytes']}\r\n\r\n"
            ).encode("ascii")
            + b"x" * fixture["received_bytes"]
        )
        reader.feed_eof()
        with self.assertRaises(asyncio.IncompleteReadError):
            await _read_http_request(reader)

    async def test_ephemeral_websocket_listener_publishes_actual_port(self) -> None:
        listener, actual_port = _bind_websocket_listener("127.0.0.1", 0)
        try:
            self.assertGreater(actual_port, 0)
            self.assertEqual(listener.getsockname()[1], actual_port)
        finally:
            listener.close()


if __name__ == "__main__":
    unittest.main()
