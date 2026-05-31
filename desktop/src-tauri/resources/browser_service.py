"""Playwright browser screenshot stream sidecar for teshi-desktop."""

from __future__ import annotations

import argparse
import asyncio
import base64
import json
import sys

from playwright.async_api import Browser, Page, async_playwright


class BrowserSession:
    def __init__(self) -> None:
        self.page: Page | None = None
        self.browser: Browser | None = None

    async def start(self) -> None:
        playwright = await async_playwright().start()
        self.browser = await playwright.chromium.launch(headless=True)
        context = await self.browser.new_context(viewport={"width": 1920, "height": 1080})
        self.page = await context.new_page()
        await self.page.goto("about:blank")

    async def navigate(self, url: str) -> None:
        if self.page is not None:
            await self.page.goto(url)

    async def screenshot_jpeg_b64(self) -> str:
        if self.page is None:
            return ""
        png = await self.page.screenshot(type="jpeg", quality=70)
        return base64.b64encode(png).decode("ascii")


async def run_server(host: str, port: int) -> None:
    import websockets

    session = BrowserSession()
    await session.start()
    clients: set = set()

    async def handler(websocket):
        clients.add(websocket)
        try:
            async for message in websocket:
                try:
                    data = json.loads(message)
                except json.JSONDecodeError:
                    continue
                if data.get("cmd") == "navigate":
                    await session.navigate(data.get("url", "about:blank"))
        finally:
            clients.discard(websocket)

    async with websockets.serve(handler, host, port):
        while True:
            if clients:
                frame = await session.screenshot_jpeg_b64()
                payload = json.dumps({"type": "frame", "data": frame})
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
    args = parser.parse_args()
    try:
        asyncio.run(run_server(args.host, args.port))
    except KeyboardInterrupt:
        sys.exit(0)


if __name__ == "__main__":
    main()
