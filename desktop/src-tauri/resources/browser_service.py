"""Browser bridge for teshi-desktop: embedded Playwright or Chrome extension."""

from __future__ import annotations

import argparse
import asyncio
import base64
import json
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import parse_qs, urlparse
from typing import Any

DEFAULT_DISCOVERY_PORT = 17373
# Extension is considered connected if heartbeat POST was received within this window.
HEARTBEAT_TTL_SEC = 8.0
EXTENSION_FRAME_WS_PATH = "/extension/frames"
FRAME_MAGIC = b"TSH1"


def paths_equal(got: str, expected: Path) -> bool:
    """Compare project roots (case-insensitive on Windows)."""
    if not got or not str(got).strip():
        return True
    try:
        a = Path(got).resolve()
        b = expected.resolve()
        if a == b:
            return True
        return str(a).casefold() == str(b).casefold()
    except OSError:
        return False

HIGHLIGHT_CONFIG = {
    "showInfo": True,
    "showStyles": True,
    "showRulers": False,
    "showExtensionLines": False,
    "contentColor": {"r": 37, "g": 99, "b": 235, "a": 0.35},
    "borderColor": {"r": 37, "g": 99, "b": 235, "a": 0.9},
}

INTERACTIVE_SELECTOR = (
    "button, [role='button'], input, input[type='submit'], select, "
    "a[href], [role='link'], textarea"
)

INTERACTIVE_EVAL = f"""() => {{
  const elements = Array.from(document.querySelectorAll({json.dumps(INTERACTIVE_SELECTOR)}));
  return elements.slice(0, 60).map(el => ({{
    tag: el.tagName.toLowerCase(),
    text: (el.innerText || el.value || el.getAttribute('aria-label') || '').trim().slice(0, 120),
    id: el.id || null,
    testId: el.getAttribute('data-testid'),
    role: el.getAttribute('role'),
    classes: el.className || null,
  }}));
}}"""


def normalize_snapshot(
    url: str,
    title: str,
    accessibility_tree: Any,
    interactive_elements: list[Any],
) -> dict[str, Any]:
    """Shared response shape for embedded and chrome modes."""
    return {
        "ok": True,
        "url": url,
        "title": title,
        "accessibility_tree": accessibility_tree,
        "interactive_elements": interactive_elements,
    }


def write_cdp_endpoint_file(
    project_root: Path,
    *,
    mode: str,
    ws_url: str,
    page_url: str,
    discovery_port: int | None = None,
    cdp_http_url: str | None = None,
    extension_connected: bool = False,
    extension_frame_ws_url: str | None = None,
) -> None:
    teshi_dir = project_root / ".teshi"
    teshi_dir.mkdir(parents=True, exist_ok=True)
    payload: dict[str, Any] = {
        "mode": mode,
        "ws_url": ws_url,
        "page_url": page_url,
        "bridge": "python",
        "extension_connected": extension_connected,
    }
    if mode == "embedded":
        payload["viewport"] = {"width": 1920, "height": 1080}
        if cdp_http_url:
            payload["http_url"] = cdp_http_url
    if mode == "chrome" and discovery_port is not None:
        payload["discovery_url"] = f"http://127.0.0.1:{discovery_port}/v1/bridge"
    if extension_frame_ws_url:
        payload["extension_frame_ws_url"] = extension_frame_ws_url
    (teshi_dir / "cdp-endpoint.json").write_text(
        json.dumps(payload, indent=2),
        encoding="utf-8",
    )


def parse_tsh1_frame(data: bytes) -> tuple[dict[str, Any], bytes] | None:
    """Parse extension binary frame: magic TSH1 + meta_len + meta JSON + JPEG."""
    if len(data) < 8 or data[:4] != FRAME_MAGIC:
        return None
    meta_len = int.from_bytes(data[4:8], "little")
    end_meta = 8 + meta_len
    if len(data) < end_meta:
        return None
    try:
        meta = json.loads(data[8:end_meta].decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return None
    if not isinstance(meta, dict):
        return None
    return meta, data[end_meta:]


def build_frame_out_sync(meta: dict[str, Any], jpeg: bytes) -> dict[str, Any]:
    """Build desktop WebSocket frame payload (base64 JPEG) off the event loop."""
    frame_out: dict[str, Any] = {
        "type": "frame",
        "data": base64.b64encode(jpeg).decode("ascii"),
        "url": str(meta.get("url", "")),
    }
    raw_tab = meta.get("tab_id")
    if raw_tab is not None:
        try:
            frame_out["tab_id"] = int(raw_tab)
        except (TypeError, ValueError):
            pass
    return frame_out


def fetch_playwright_cdp_endpoint(cdp_port: int) -> dict[str, Any]:
    url = f"http://127.0.0.1:{cdp_port}/json/version"
    with urllib.request.urlopen(url, timeout=5) as response:
        payload = json.loads(response.read().decode("utf-8"))
    return {
        "ws_url": payload.get("webSocketDebuggerUrl", ""),
        "http_url": f"http://127.0.0.1:{cdp_port}",
    }


# --- Embedded (Playwright) backend ---


class EmbeddedSession:
    def __init__(self) -> None:
        self.page = None
        self.browser = None
        self.context = None
        self.playwright = None
        self.cdp_session = None

    async def start(self, cdp_port: int) -> None:
        from playwright.async_api import async_playwright

        self.playwright = await async_playwright().start()
        self.browser = await self.playwright.chromium.launch(
            headless=True,
            args=[f"--remote-debugging-port={cdp_port}"],
        )
        self.context = await self.browser.new_context(
            viewport={"width": 1920, "height": 1080}
        )
        self.page = await self.context.new_page()
        await self.page.goto("about:blank")
        self.cdp_session = await self.context.new_cdp_session(self.page)

    def current_url(self) -> str:
        if self.page is None:
            return "about:blank"
        return self.page.url

    async def navigate(self, url: str) -> None:
        if self.page is not None:
            await self.page.goto(url, wait_until="domcontentloaded")

    async def screenshot_jpeg_b64(self) -> str:
        if self.page is None:
            return ""
        png = await self.page.screenshot(type="jpeg", quality=70)
        return base64.b64encode(png).decode("ascii")

    async def clear_highlight(self) -> None:
        if self.cdp_session is None:
            return
        await self.cdp_session.send("Overlay.hideHighlight", {})

    async def highlight_selector(self, selector: str) -> dict[str, Any]:
        if self.page is None or self.cdp_session is None:
            return {"ok": False, "error": "browser not ready"}

        await self.clear_highlight()
        locator = self.page.locator(selector)
        count = await locator.count()
        if count == 0:
            return {"ok": False, "error": f"selector matched no elements: {selector}"}
        if count > 1:
            return {
                "ok": False,
                "error": f"selector matched {count} elements; refine selector",
            }

        object_result = await self.cdp_session.send(
            "Runtime.evaluate",
            {
                "expression": f"document.querySelector({json.dumps(selector)})",
                "returnByValue": False,
            },
        )
        object_id = object_result.get("result", {}).get("objectId")
        if not object_id:
            return {"ok": False, "error": "could not evaluate selector in page context"}

        node_result = await self.cdp_session.send(
            "DOM.requestNode",
            {"objectId": object_id},
        )
        node_id = node_result.get("nodeId")
        if not node_id:
            return {"ok": False, "error": "could not resolve node id"}

        await self.cdp_session.send(
            "Overlay.highlightNode",
            {"highlightConfig": HIGHLIGHT_CONFIG, "nodeId": node_id},
        )
        box = await locator.bounding_box()
        return {
            "ok": True,
            "selector": selector,
            "node_id": node_id,
            "bounding_box": box,
        }

    async def get_page_snapshot(self) -> dict[str, Any]:
        if self.page is None:
            return {"ok": False, "error": "browser not ready"}

        title = await self.page.title()
        url = self.page.url
        try:
            tree = await self.page.accessibility.snapshot(interesting_only=False)
        except Exception as exc:  # noqa: BLE001
            tree = {"error": str(exc)}

        buttons = await self.page.evaluate(INTERACTIVE_EVAL)
        return normalize_snapshot(url, title, tree, buttons)


async def handle_embedded_command(
    session: EmbeddedSession, data: dict[str, Any]
) -> dict[str, Any]:
    cmd = data.get("cmd")
    request_id = data.get("request_id")

    if cmd == "navigate":
        await session.navigate(data.get("url", "about:blank"))
        return {"type": "response", "request_id": request_id, "ok": True}

    if cmd == "highlight_selector":
        result = await session.highlight_selector(data.get("selector", ""))
        return {"type": "response", "request_id": request_id, **result}

    if cmd == "clear_highlight":
        await session.clear_highlight()
        return {"type": "response", "request_id": request_id, "ok": True}

    if cmd == "get_page_snapshot":
        snapshot = await session.get_page_snapshot()
        return {"type": "response", "request_id": request_id, **snapshot}

    return {
        "type": "response",
        "request_id": request_id,
        "ok": False,
        "error": f"unknown cmd: {cmd}",
    }


async def run_embedded(
    host: str,
    port: int,
    cdp_port: int,
    project_root: Path | None,
) -> None:
    import websockets

    session = EmbeddedSession()
    await session.start(cdp_port)
    cdp_meta: dict[str, Any] = {}
    if project_root is not None:
        try:
            cdp_meta = fetch_playwright_cdp_endpoint(cdp_port)
            write_cdp_endpoint_file(
                project_root,
                mode="embedded",
                ws_url=f"ws://{host}:{port}",
                page_url=session.current_url(),
                cdp_http_url=cdp_meta.get("http_url"),
                extension_connected=False,
            )
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
            print(f"warning: failed to write cdp-endpoint.json: {exc}", file=sys.stderr)

    clients: set[Any] = set()

    async def handler(websocket: Any) -> None:
        clients.add(websocket)
        try:
            async for message in websocket:
                try:
                    data = json.loads(message)
                except json.JSONDecodeError:
                    continue
                if "cmd" in data:
                    reply = await handle_embedded_command(session, data)
                    await websocket.send(json.dumps(reply))
                    if (
                        project_root is not None
                        and data.get("cmd") == "navigate"
                        and reply.get("ok")
                    ):
                        write_cdp_endpoint_file(
                            project_root,
                            mode="embedded",
                            ws_url=f"ws://{host}:{port}",
                            page_url=session.current_url(),
                            cdp_http_url=cdp_meta.get("http_url"),
                        )
        finally:
            clients.discard(websocket)

    async with websockets.serve(handler, host, port):
        while True:
            if clients:
                frame = await session.screenshot_jpeg_b64()
                payload = json.dumps(
                    {
                        "type": "frame",
                        "data": frame,
                        "url": session.current_url(),
                    }
                )
                dead = []
                for ws in clients:
                    try:
                        await ws.send(payload)
                    except Exception:
                        dead.append(ws)
                for ws in dead:
                    clients.discard(ws)
            await asyncio.sleep(0.125)


# --- Chrome extension backend ---


class ChromeBridge:
    """Chrome mode: extension talks HTTP heartbeat; agents use WebSocket."""

    def __init__(
        self,
        project_root: Path,
        ws_url: str,
        discovery_port: int,
        extension_frame_ws_url: str,
        frame_callback: Any | None = None,
        event_callback: Any | None = None,
    ) -> None:
        self.project_root = project_root.resolve()
        self.ws_url = ws_url
        self.discovery_port = discovery_port
        self.extension_frame_ws_url = extension_frame_ws_url
        self.page_url = ""
        self.page_title = ""
        self.active_tab_id: int | None = None
        self.tabs: list[dict[str, Any]] = []
        self.last_frame_error = ""
        self._last_frame: dict[str, Any] | None = None
        self.last_frame_at: float | None = None
        self.last_heartbeat = 0.0
        self._cmd_queue: list[dict[str, Any]] = []
        self._pending: dict[str, asyncio.Future[dict[str, Any]]] = {}
        self._pending_stream_restart = False
        self._frame_callback = frame_callback
        self._event_callback = event_callback
        self._deprecated_json_frame_warned = False

    def extension_alive(self) -> bool:
        return (time.monotonic() - self.last_heartbeat) < HEARTBEAT_TTL_SEC

    def bridge_info(self) -> dict[str, Any]:
        last_frame_age_ms: int | None = None
        if self.last_frame_at is not None:
            last_frame_age_ms = int((time.monotonic() - self.last_frame_at) * 1000)
        return {
            "ws_url": self.ws_url,
            "extension_frame_ws_url": self.extension_frame_ws_url,
            "project_root": str(self.project_root),
            "mode": "chrome",
            "transport": "http-heartbeat+ws-screencast",
            "extension_connected": self.extension_alive(),
            "page_url": self.page_url,
            "title": self.page_title,
            "active_tab_id": self.active_tab_id,
            "tabs": self.tabs,
            "last_frame_error": self.last_frame_error,
            "last_frame_age_ms": last_frame_age_ms,
        }

    def write_endpoint(self) -> None:
        write_cdp_endpoint_file(
            self.project_root,
            mode="chrome",
            ws_url=self.ws_url,
            page_url=self.page_url or "about:blank",
            discovery_port=self.discovery_port,
            extension_connected=self.extension_alive(),
            extension_frame_ws_url=self.extension_frame_ws_url,
        )

    async def handle_heartbeat(self, payload: dict[str, Any]) -> dict[str, Any]:
        got = str(payload.get("project_root", ""))
        if not paths_equal(got, self.project_root):
            return {
                "ok": False,
                "error": f"project_root mismatch: expected {self.project_root}",
            }
        self.last_heartbeat = time.monotonic()
        self.page_url = str(payload.get("url", self.page_url))
        self.page_title = str(payload.get("title", self.page_title))
        raw_active = payload.get("active_tab_id")
        if raw_active is not None:
            try:
                self.active_tab_id = int(raw_active)
            except (TypeError, ValueError):
                pass
        raw_tabs = payload.get("tabs")
        if isinstance(raw_tabs, list):
            self.tabs = raw_tabs
        frame_error = payload.get("frame_error")
        if isinstance(frame_error, str) and frame_error.strip():
            self.last_frame_error = frame_error.strip()
        self.write_endpoint()
        pending_cmd = self._cmd_queue.pop(0) if self._cmd_queue else None
        stream_restart = self._pending_stream_restart
        self._pending_stream_restart = False
        return {
            "ok": True,
            "cmd": pending_cmd,
            "stream_restart": stream_restart,
            # Legacy alias for older extension builds.
            "force_capture": stream_restart,
        }

    def _apply_frame_state(self, frame_out: dict[str, Any]) -> None:
        """Update bridge metadata and cache the latest frame (sync, HTTP-fast)."""
        if frame_out.get("url"):
            self.page_url = str(frame_out["url"])
        raw_tab = frame_out.get("tab_id")
        if raw_tab is not None:
            try:
                self.active_tab_id = int(raw_tab)
            except (TypeError, ValueError):
                pass
        self._last_frame = frame_out
        self.last_frame_at = time.monotonic()
        self.last_frame_error = ""
        self.write_endpoint()

    async def _emit_frame(self, frame_out: dict[str, Any]) -> None:
        self._apply_frame_state(frame_out)
        if self._frame_callback is not None:
            await self._frame_callback(frame_out)

    def _schedule_frame_broadcast(self, frame_out: dict[str, Any]) -> None:
        """Push frames to desktop WebSocket clients without blocking HTTP /response."""
        if self._frame_callback is None:
            return
        asyncio.create_task(self._frame_callback(frame_out))

    async def _emit_stream_event(self, event: dict[str, Any]) -> None:
        if self._event_callback is not None:
            await self._event_callback(event)

    def _schedule_stream_event(self, event: dict[str, Any]) -> None:
        if self._event_callback is None:
            return
        asyncio.create_task(self._emit_stream_event(event))

    def validate_stream_hello(self, payload: dict[str, Any]) -> dict[str, Any]:
        got = str(payload.get("project_root", ""))
        if not paths_equal(got, self.project_root):
            return {
                "type": "stream_hello_ack",
                "ok": False,
                "error": f"project_root mismatch: expected {self.project_root}",
            }
        self.last_heartbeat = time.monotonic()
        return {"type": "stream_hello_ack", "ok": True}

    async def handle_extension_binary(self, data: bytes) -> None:
        parsed = parse_tsh1_frame(data)
        if parsed is None:
            return
        meta, jpeg = parsed
        if not jpeg:
            return
        frame_out = await asyncio.to_thread(build_frame_out_sync, meta, jpeg)
        self.last_heartbeat = time.monotonic()
        self._apply_frame_state(frame_out)
        self._schedule_frame_broadcast(frame_out)

    async def handle_extension_response(self, payload: dict[str, Any]) -> dict[str, Any]:
        if payload.get("type") == "frame_error":
            self.last_frame_error = str(payload.get("error", "screenshot failed"))
            self.write_endpoint()
            self._schedule_stream_event(
                {
                    "type": "frame_error",
                    "error": self.last_frame_error,
                }
            )
            return {"ok": True}

        if payload.get("type") == "frame":
            if not self._deprecated_json_frame_warned:
                self._deprecated_json_frame_warned = True
                print(
                    "warning: JSON frame on POST /v1/bridge/response is deprecated; "
                    "use extension WebSocket screencast (/extension/frames)",
                    file=sys.stderr,
                )
            data_field = payload.get("data", "")
            if isinstance(data_field, str) and len(data_field) > 4096:
                return {"ok": True, "deprecated": True, "ignored": True}
            self.last_heartbeat = time.monotonic()
            self.page_title = str(payload.get("title", self.page_title))
            frame_out = {
                "type": "frame",
                "data": data_field,
                "url": str(payload.get("url", self.page_url)),
            }
            raw_tab = payload.get("tab_id")
            if raw_tab is not None:
                try:
                    frame_out["tab_id"] = int(raw_tab)
                except (TypeError, ValueError):
                    pass
            self._apply_frame_state(frame_out)
            self._schedule_frame_broadcast(frame_out)
            return {"ok": True}

        request_id = payload.get("request_id")
        if request_id:
            fut = self._pending.pop(str(request_id), None)
            if fut and not fut.done():
                fut.set_result(payload)
            if payload.get("cmd") == "get_page_snapshot" and payload.get("ok"):
                self.page_url = str(payload.get("url", self.page_url))
                self.page_title = str(payload.get("title", self.page_title))
                self.write_endpoint()
        return {"ok": True}

    def queue_command_front(self, cmd: str, **fields: Any) -> str:
        """Enqueue a command for the next extension heartbeat (front of queue)."""
        request_id = str(fields.pop("request_id", f"cmd-{time.monotonic()}"))
        entry: dict[str, Any] = {
            "type": "cmd",
            "request_id": request_id,
            "cmd": cmd,
        }
        entry.update(fields)
        self._cmd_queue.insert(0, entry)
        return request_id

    async def handle_activate_tab_http(self, payload: dict[str, Any]) -> dict[str, Any]:
        got = str(payload.get("project_root", ""))
        if not paths_equal(got, self.project_root):
            return {
                "ok": False,
                "error": f"project_root mismatch: expected {self.project_root}",
            }
        raw_tab = payload.get("tab_id")
        if raw_tab is None:
            return {"ok": False, "error": "tab_id is required"}
        try:
            tab_id = int(raw_tab)
        except (TypeError, ValueError):
            return {"ok": False, "error": "tab_id must be an integer"}
        self.active_tab_id = tab_id
        self.write_endpoint()
        self.queue_command_front("activate_tab", tab_id=tab_id)
        self._pending_stream_restart = True
        return {"ok": True}

    async def handle_capture_now_http(self, payload: dict[str, Any]) -> dict[str, Any]:
        got = str(payload.get("project_root", ""))
        if not paths_equal(got, self.project_root):
            return {
                "ok": False,
                "error": f"project_root mismatch: expected {self.project_root}",
            }
        self._pending_stream_restart = True
        return {"ok": True}

    async def forward_command(self, data: dict[str, Any]) -> dict[str, Any]:
        request_id = str(data.get("request_id") or "")
        if not self.extension_alive():
            return {
                "type": "response",
                "request_id": request_id,
                "ok": False,
                "error": (
                    "Chrome extension not connected (no heartbeat). Keep the "
                    "feedback.enhook.com tab active in Chrome and ensure "
                    "teshi-bridge is loaded — it polls every second while the bridge runs."
                ),
            }

        loop = asyncio.get_running_loop()
        fut: asyncio.Future[dict[str, Any]] = loop.create_future()
        self._pending[request_id] = fut
        queued: dict[str, Any] = {
            "type": "cmd",
            "request_id": request_id,
            "cmd": data.get("cmd"),
            "selector": data.get("selector"),
            "url": data.get("url"),
        }
        if data.get("tab_id") is not None:
            queued["tab_id"] = data.get("tab_id")
        self._cmd_queue.append(queued)
        try:
            return await asyncio.wait_for(fut, timeout=45.0)
        except asyncio.TimeoutError:
            self._pending.pop(request_id, None)
            self._cmd_queue = [c for c in self._cmd_queue if c.get("request_id") != request_id]
            return {
                "type": "response",
                "request_id": request_id,
                "ok": False,
                "error": "extension did not respond in time (heartbeat may have stalled)",
            }


def _http_response(status: int, body: bytes, content_type: str = "application/json") -> bytes:
    reason = "OK" if status == 200 else "Not Found"
    header = (
        f"HTTP/1.1 {status} {reason}\r\n"
        f"Content-Type: {content_type}\r\n"
        "Access-Control-Allow-Origin: *\r\n"
        "Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n"
        "Access-Control-Allow-Headers: Content-Type, X-Project-Root, X-Tab-Id, X-Url\r\n"
        f"Content-Length: {len(body)}\r\n"
        "Connection: close\r\n"
        "\r\n"
    ).encode("ascii")
    return header + body


async def _read_http_request(
    reader: asyncio.StreamReader,
) -> tuple[str, dict[str, str], bytes]:
    request_line = (await reader.readline()).decode("utf-8", errors="ignore").strip()
    headers: dict[str, str] = {}
    while True:
        line = await reader.readline()
        if line in (b"\r\n", b"\n", b""):
            break
        decoded = line.decode("utf-8", errors="ignore").strip()
        if ":" in decoded:
            key, value = decoded.split(":", 1)
            headers[key.strip().lower()] = value.strip()
    length = int(headers.get("content-length", "0") or "0")
    body = await reader.read(length) if length > 0 else b""
    return request_line, headers, body


async def run_http_discovery(
    bridge: ChromeBridge, host: str, discovery_port: int
) -> None:
    async def handle_client(
        reader: asyncio.StreamReader, writer: asyncio.StreamWriter
    ) -> None:
        try:
            request_line, _headers, body = await _read_http_request(reader)
            parts = request_line.split()
            method = parts[0].upper() if parts else ""
            path = parts[1] if len(parts) > 1 else ""

            if method == "OPTIONS":
                writer.write(_http_response(200, b""))
            elif method == "GET" and path == "/v1/bridge":
                payload = json.dumps(bridge.bridge_info()).encode("utf-8")
                writer.write(_http_response(200, payload))
            elif method == "POST" and path == "/v1/bridge/heartbeat":
                data = json.loads(body.decode("utf-8") or "{}")
                result = await bridge.handle_heartbeat(data)
                writer.write(_http_response(200, json.dumps(result).encode("utf-8")))
            elif method == "POST" and path == "/v1/bridge/response":
                text = body.decode("utf-8") or "{}"
                if len(body) > 65536:
                    data = await asyncio.to_thread(json.loads, text)
                else:
                    data = json.loads(text)
                result = await bridge.handle_extension_response(data)
                writer.write(_http_response(200, json.dumps(result).encode("utf-8")))
            elif method == "POST" and path == "/v1/bridge/activate_tab":
                data = json.loads(body.decode("utf-8") or "{}")
                result = await bridge.handle_activate_tab_http(data)
                writer.write(_http_response(200, json.dumps(result).encode("utf-8")))
            elif method == "POST" and path == "/v1/bridge/capture_now":
                data = json.loads(body.decode("utf-8") or "{}")
                result = await bridge.handle_capture_now_http(data)
                writer.write(_http_response(200, json.dumps(result).encode("utf-8")))
            else:
                writer.write(_http_response(404, b"{}"))
            await writer.drain()
        finally:
            writer.close()
            await writer.wait_closed()

    server = await asyncio.start_server(handle_client, host, discovery_port)
    async with server:
        await server.serve_forever()


def _websocket_path(websocket: Any) -> str:
    request = getattr(websocket, "request", None)
    if request is not None:
        return str(getattr(request, "path", "/") or "/")
    return str(getattr(websocket, "path", "/") or "/")


async def run_chrome(
    host: str,
    port: int,
    discovery_port: int,
    project_root: Path,
) -> None:
    import websockets

    ws_url = f"ws://{host}:{port}"
    extension_frame_ws_url = f"ws://{host}:{port}{EXTENSION_FRAME_WS_PATH}"
    clients: set[Any] = set()

    async def broadcast_ws_message(message: dict[str, Any]) -> None:
        if not clients:
            return
        payload = json.dumps(message)
        dead: list[Any] = []

        async def send_one(ws: Any) -> None:
            try:
                await ws.send(payload)
            except Exception:
                dead.append(ws)

        await asyncio.gather(*(send_one(ws) for ws in list(clients)))
        for ws in dead:
            clients.discard(ws)

    async def broadcast_frame(frame_payload: dict[str, Any]) -> None:
        await broadcast_ws_message(frame_payload)

    bridge = ChromeBridge(
        project_root,
        ws_url,
        discovery_port,
        extension_frame_ws_url,
        frame_callback=broadcast_frame,
        event_callback=broadcast_ws_message,
    )
    bridge.write_endpoint()

    http_task = asyncio.create_task(run_http_discovery(bridge, host, discovery_port))

    async def handle_extension_websocket(websocket: Any) -> None:
        stream_authenticated = False
        try:
            async for message in websocket:
                if isinstance(message, str):
                    try:
                        data = json.loads(message)
                    except json.JSONDecodeError:
                        continue
                    if data.get("type") == "stream_hello":
                        ack = bridge.validate_stream_hello(data)
                        stream_authenticated = bool(ack.get("ok"))
                        await websocket.send(json.dumps(ack))
                    elif data.get("type") == "frame_error":
                        bridge.last_frame_error = str(
                            data.get("error", "extension stream error")
                        )
                        bridge.write_endpoint()
                        bridge._schedule_stream_event(
                            {
                                "type": "frame_error",
                                "error": bridge.last_frame_error,
                            }
                        )
                elif isinstance(message, bytes) and stream_authenticated:
                    await bridge.handle_extension_binary(message)
        except Exception as exc:  # noqa: BLE001
            print(f"extension frame websocket closed: {exc}", file=sys.stderr)

    async def handle_desktop_websocket(websocket: Any) -> None:
        clients.add(websocket)
        if bridge._last_frame is not None:
            try:
                await websocket.send(json.dumps(bridge._last_frame))
            except Exception:
                clients.discard(websocket)
        try:
            async for message in websocket:
                try:
                    data = json.loads(message)
                except json.JSONDecodeError:
                    continue
                if data.get("type") == "frame" and data.get("data"):
                    frame_out: dict[str, Any] = {
                        "type": "frame",
                        "data": data.get("data", ""),
                        "url": str(data.get("url", bridge.page_url)),
                    }
                    raw_tab = data.get("tab_id")
                    if raw_tab is not None:
                        try:
                            frame_out["tab_id"] = int(raw_tab)
                        except (TypeError, ValueError):
                            pass
                    await bridge._emit_frame(frame_out)
                    continue
                if "cmd" in data:
                    reply = await bridge.forward_command(data)
                    await websocket.send(json.dumps(reply))
        finally:
            clients.discard(websocket)

    async def connection_handler(websocket: Any) -> None:
        path = _websocket_path(websocket)
        if path == EXTENSION_FRAME_WS_PATH:
            await handle_extension_websocket(websocket)
            return
        await handle_desktop_websocket(websocket)

    async with websockets.serve(connection_handler, host, port):
        await asyncio.gather(http_task, asyncio.Future())


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--mode", choices=("embedded", "chrome"), default="embedded")
    parser.add_argument("--cdp-port", type=int, default=0)
    parser.add_argument("--discovery-port", type=int, default=DEFAULT_DISCOVERY_PORT)
    parser.add_argument("--project-root", default="")
    args = parser.parse_args()
    project_root = Path(args.project_root) if args.project_root else None

    try:
        if args.mode == "chrome":
            if project_root is None:
                print("chrome mode requires --project-root", file=sys.stderr)
                sys.exit(1)
            asyncio.run(
                run_chrome(args.host, args.port, args.discovery_port, project_root)
            )
        else:
            if args.cdp_port <= 0:
                print("embedded mode requires --cdp-port", file=sys.stderr)
                sys.exit(1)
            asyncio.run(
                run_embedded(args.host, args.port, args.cdp_port, project_root)
            )
    except KeyboardInterrupt:
        sys.exit(0)


if __name__ == "__main__":
    main()
