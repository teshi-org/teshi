"""Regression tests for the browser bridge's small HTTP server."""

from __future__ import annotations

import asyncio
import contextlib
import json
import os
import socket
import sys
import tempfile
import threading
import unittest
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen
from pathlib import Path
from unittest.mock import patch

RESOURCES = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RESOURCES))

from browser_service import (  # noqa: E402
    BrowserSessionBroker,
    ChromeBridge,
    _bind_websocket_listener,
    _read_http_request,
    credential_free_discovery,
    embedded_browser_headless,
    handle_embedded_command,
    paths_equal,
    run_http_discovery,
    write_cdp_endpoint_file,
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
    async def test_python_discovery_get_is_public_but_post_keeps_extension_compatibility(self) -> None:
        with tempfile.TemporaryDirectory(prefix="teshi-discovery-http-") as root:
            project_root = Path(root)
            with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
                probe.bind(("127.0.0.1", 0))
                port = int(probe.getsockname()[1])
            bridge = ChromeBridge(
                project_root,
                "ws://127.0.0.1:20254/?token=secret-token",
                port,
                "ws://127.0.0.1:20254/extension/frames?token=secret-token",
            )
            server_task = asyncio.create_task(
                run_http_discovery(bridge, "127.0.0.1", port, "command-token")
            )

            def request(method: str, origin: str | None = None) -> tuple[int, dict]:
                request = Request(
                    f"http://127.0.0.1:{port}/v1/bridge",
                    data=b"{}" if method == "POST" else None,
                    method=method,
                )
                if method == "POST":
                    request.add_header("Content-Type", "application/json")
                if origin is not None:
                    request.add_header("Origin", origin)
                try:
                    with urlopen(request, timeout=2) as response:
                        return response.status, json.loads(response.read())
                except HTTPError as error:
                    return error.code, {}

            async def wait_for_request(method: str, origin: str | None = None) -> tuple[int, dict]:
                deadline = asyncio.get_running_loop().time() + 5
                while True:
                    try:
                        return await asyncio.to_thread(request, method, origin)
                    except URLError:
                        if asyncio.get_running_loop().time() >= deadline:
                            raise
                        await asyncio.sleep(0.05)

            try:
                public_status, public = await wait_for_request("GET")
                self.assertEqual(public_status, 200)
                self.assertNotIn("token=", json.dumps(public))
                self.assertNotIn("project_root", public)

                extension_origin = "chrome-extension://" + ("a" * 32)
                trusted_status, trusted = await wait_for_request(
                    "POST", extension_origin
                )
                self.assertEqual(trusted_status, 200)
                self.assertIn("token=secret-token", trusted["ws_url"])
                self.assertEqual(trusted["project_root"], str(project_root))

                missing_status, _ = await wait_for_request("POST")
                self.assertEqual(missing_status, 403)
                hostile_status, hostile = await wait_for_request(
                    "POST", "https://ordinary.example.test"
                )
                self.assertEqual(hostile_status, 403)
                self.assertEqual(hostile, {})
            finally:
                server_task.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await asyncio.wait_for(server_task, timeout=5)

    def test_public_discovery_projection_removes_bearer_query_parameters(self) -> None:
        public = credential_free_discovery(
            {
                "ws_url": "ws://127.0.0.1:20254/?token=secret&channel=control",
                "extension_frame_ws_url": (
                    "ws://127.0.0.1:20254/extension/frames?token=secret"
                ),
                "project_root": r"D:\private\project",
                "bridge": "python",
            }
        )
        self.assertEqual(
            public["ws_url"], "ws://127.0.0.1:20254/?channel=control"
        )
        self.assertEqual(
            public["extension_frame_ws_url"],
            "ws://127.0.0.1:20254/extension/frames",
        )
        self.assertNotIn("project_root", public)
        self.assertEqual(public["bridge"], "python")

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

    def test_cdp_endpoint_file_is_never_observed_partially(self) -> None:
        with tempfile.TemporaryDirectory(prefix="teshi-endpoint-test-") as root:
            project_root = Path(root)
            write_cdp_endpoint_file(
                project_root,
                mode="chrome",
                ws_url="ws://127.0.0.1:20001/browser",
                page_url="http://127.0.0.1:20001/",
                discovery_port=17373,
            )
            endpoint_path = project_root / ".teshi" / "cdp-endpoint.json"
            errors: list[str] = []
            done = threading.Event()

            def writer() -> None:
                try:
                    for index in range(64):
                        write_cdp_endpoint_file(
                            project_root,
                            mode="chrome",
                            ws_url=f"ws://127.0.0.1:{20001 + index}/browser",
                            page_url=f"http://127.0.0.1:{20001 + index}/",
                            discovery_port=17373,
                        )
                except Exception as error:  # pragma: no cover - assertion below
                    errors.append(f"writer: {error}")
                finally:
                    done.set()

            def reader() -> None:
                while not done.is_set():
                    try:
                        value = json.loads(endpoint_path.read_text(encoding="utf-8"))
                        if not isinstance(value, dict) or not value.get("ws_url"):
                            errors.append("reader: incomplete endpoint payload")
                    except PermissionError:
                        # Windows may briefly deny a new open while the old
                        # endpoint handle is being atomically replaced.
                        continue
                    except (OSError, json.JSONDecodeError) as error:
                        errors.append(f"reader: {error}")

            writer_thread = threading.Thread(target=writer)
            reader_thread = threading.Thread(target=reader)
            reader_thread.start()
            writer_thread.start()
            writer_thread.join(timeout=10)
            done.set()
            reader_thread.join(timeout=10)
            self.assertFalse(writer_thread.is_alive())
            self.assertFalse(reader_thread.is_alive())
            self.assertEqual(errors, [])

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
