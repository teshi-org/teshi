"""Playwright browser screenshot stream sidecar for teshi-desktop."""

from __future__ import annotations

import argparse
import asyncio
import base64
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

from playwright.async_api import Browser, BrowserContext, Page, async_playwright


HIGHLIGHT_CONFIG = {
    "showInfo": True,
    "showStyles": True,
    "showRulers": False,
    "showExtensionLines": False,
    "contentColor": {"r": 37, "g": 99, "b": 235, "a": 0.35},
    "borderColor": {"r": 37, "g": 99, "b": 235, "a": 0.9},
}


class BrowserSession:
    def __init__(self) -> None:
        self.page: Page | None = None
        self.browser: Browser | None = None
        self.context: BrowserContext | None = None
        self.playwright = None
        self.cdp_session = None
        self._highlighted_node_id: int | None = None

    async def start(self, cdp_port: int) -> None:
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
        self._highlighted_node_id = None

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
        self._highlighted_node_id = node_id
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
        except Exception as exc:  # noqa: BLE001 - surface to agent as structured error
            tree = {"error": str(exc)}

        buttons = await self.page.eval_on_selector_all(
            "button, [role='button'], input[type='submit'], a[role='button']",
            """elements => elements.slice(0, 40).map(el => ({
                tag: el.tagName.toLowerCase(),
                text: (el.innerText || el.value || el.getAttribute('aria-label') || '').trim().slice(0, 120),
                id: el.id || null,
                testId: el.getAttribute('data-testid'),
                role: el.getAttribute('role'),
                classes: el.className || null,
            }))""",
        )
        return {
            "ok": True,
            "url": url,
            "title": title,
            "accessibility_tree": tree,
            "interactive_elements": buttons,
        }


def fetch_cdp_endpoint(cdp_port: int) -> dict[str, Any]:
    url = f"http://127.0.0.1:{cdp_port}/json/version"
    with urllib.request.urlopen(url, timeout=5) as response:
        payload = json.loads(response.read().decode("utf-8"))
    return {
        "ws_url": payload.get("webSocketDebuggerUrl", ""),
        "http_url": f"http://127.0.0.1:{cdp_port}",
    }


def write_cdp_endpoint_file(
    project_root: Path,
    cdp_port: int,
    page_url: str,
) -> None:
    teshi_dir = project_root / ".teshi"
    teshi_dir.mkdir(parents=True, exist_ok=True)
    endpoint = fetch_cdp_endpoint(cdp_port)
    endpoint["page_url"] = page_url
    endpoint["viewport"] = {"width": 1920, "height": 1080}
    (teshi_dir / "cdp-endpoint.json").write_text(
        json.dumps(endpoint, indent=2),
        encoding="utf-8",
    )


async def handle_command(session: BrowserSession, data: dict[str, Any]) -> dict[str, Any]:
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


async def run_server(
    host: str,
    port: int,
    cdp_port: int,
    project_root: Path | None,
) -> None:
    import websockets

    session = BrowserSession()
    await session.start(cdp_port)
    if project_root is not None:
        try:
            write_cdp_endpoint_file(project_root, cdp_port, session.current_url())
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
            print(f"warning: failed to write cdp-endpoint.json: {exc}", file=sys.stderr)

    clients: set[Any] = set()

    async def handler(websocket):
        clients.add(websocket)
        try:
            async for message in websocket:
                try:
                    data = json.loads(message)
                except json.JSONDecodeError:
                    continue
                if "cmd" in data:
                    reply = await handle_command(session, data)
                    await websocket.send(json.dumps(reply))
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


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--cdp-port", type=int, required=True)
    parser.add_argument("--project-root", default="")
    args = parser.parse_args()
    project_root = Path(args.project_root) if args.project_root else None
    try:
        asyncio.run(
            run_server(args.host, args.port, args.cdp_port, project_root)
        )
    except KeyboardInterrupt:
        sys.exit(0)


if __name__ == "__main__":
    main()
