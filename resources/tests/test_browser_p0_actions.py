"""Real Chromium acceptance for canonical P0 DOM/pointer/input/wait behavior."""

from __future__ import annotations

import socket
import sys
import unittest
from pathlib import Path

RESOURCES = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RESOURCES))

from browser_service import EmbeddedSession  # noqa: E402


def free_port() -> int:
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    port = int(listener.getsockname()[1])
    listener.close()
    return port


class BrowserP0ActionTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.session = EmbeddedSession()
        await self.session.start(free_port())
        await self.session.page.set_content(
            """
            <button id="dom" onclick="this.dataset.count=String(Number(this.dataset.count||0)+1);setTimeout(()=>document.querySelector('#status').textContent='ready',80)">DOM</button>
            <button id="pointer" onclick="this.dataset.hit='yes'">Pointer</button>
            <input id="name"><select id="choice"><option value="a">A</option><option value="b">B</option></select>
            <input id="key"><div id="status">waiting</div>
            """
        )

    async def asyncTearDown(self) -> None:
        if self.session.browser is not None:
            await self.session.browser.close()
        if self.session.playwright is not None:
            await self.session.playwright.stop()

    async def test_dom_pointer_text_select_key_and_reactive_waits(self) -> None:
        dom = await self.session.execute_locator("#dom", "click", timeout_ms=2000)
        self.assertTrue(dom["ok"])
        waited = await self.session.wait_for_browser_condition(
            {"kind": "visible_text", "text": "ready"}, 2000, "#dom", None
        )
        self.assertTrue(waited["ok"])

        pointer = await self.session.execute_locator(
            "#pointer", "pointer_click", timeout_ms=2000, focus=True
        )
        self.assertTrue(pointer["ok"])
        self.assertEqual(
            await self.session.page.locator("#pointer").get_attribute("data-hit"),
            "yes",
        )

        self.assertTrue(
            (await self.session.execute_locator("#name", "fill", "Ada"))["ok"]
        )
        self.assertTrue(
            (await self.session.execute_locator("#name", "type", " Lovelace"))["ok"]
        )
        self.assertEqual(await self.session.page.locator("#name").input_value(), "Ada Lovelace")
        self.assertTrue(
            (await self.session.execute_locator("#choice", "select", "b"))["ok"]
        )
        self.assertEqual(await self.session.page.locator("#choice").input_value(), "b")
        self.assertTrue(
            (await self.session.execute_locator("#key", "press_key", "Enter"))["ok"]
        )

        timeout = await self.session.wait_for_browser_condition(
            {"kind": "visible_text", "text": "never-present"},
            100,
            "#dom",
            None,
        )
        self.assertFalse(timeout["ok"])
        self.assertEqual(timeout["code"], "browser_wait_timeout")


if __name__ == "__main__":
    unittest.main()
