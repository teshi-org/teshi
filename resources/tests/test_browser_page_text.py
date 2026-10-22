"""Exercise the Chrome/embedded collector against a real isolated Chromium DOM."""

import asyncio
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

from playwright.sync_api import sync_playwright

RESOURCES = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RESOURCES))
from browser_page_text import PAGE_TEXT_JS


class PageTextTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.playwright = sync_playwright().start()
        cls.browser = cls.playwright.chromium.launch()

    @classmethod
    def tearDownClass(cls):
        cls.browser.close()
        cls.playwright.stop()

    def setUp(self):
        self.page = self.browser.new_page()
        self.addCleanup(self.page.close)

    def scan(self, **options):
        return self.page.evaluate(
            "(options) => {" + PAGE_TEXT_JS + "; return collectPageText(options); }",
            options,
        )

    def test_extension_and_embedded_use_identical_collectors(self):
        source = RESOURCES.parent / "extension/teshi-bridge/page-text.js"
        self.assertEqual(PAGE_TEXT_JS.strip(), source.read_text(encoding="utf-8").strip())

    def test_static_text_hidden_content_and_forms(self):
        self.page.set_content('''<article><h1>Report</h1><p>First <b>paragraph</b>.</p>
          <p style="display:none">Hidden display</p><p hidden>Hidden attribute</p>
          <p style="opacity:0">Hidden opacity</p><script>Script secret</script>
          <input aria-label="Search" value="visible value">
          <input type="password" value="password secret">
          <input type="hidden" value="hidden secret">
          <textarea>Draft</textarea><select><option>Unused</option><option selected>Chosen</option></select>
          <button disabled>Save</button></article>''')
        result = self.scan()
        self.assertTrue(result["ok"])
        self.assertIn("Report\nFirst paragraph.", result["text"])
        for omitted in ("Hidden", "Script secret", "password secret", "hidden secret", "Unused"):
            self.assertNotIn(omitted, result["text"])
        for included in ("Search", "visible value", "Draft", "Chosen", "[DISABLED] Save"):
            self.assertIn(included, result["text"])

    def test_shadow_slots_frames_and_offscreen_text(self):
        self.page.set_content('''<div id="host"><span>Slotted text</span></div>
          <iframe srcdoc="<p>Frame text</p>"></iframe>
          <p style="margin-top:9000px">Below viewport</p>''')
        self.page.evaluate('''() => {
          document.querySelector('#host').attachShadow({mode:'open'}).innerHTML =
            '<p>Shadow text</p><slot></slot>';
        }''')
        self.page.locator("iframe").content_frame.locator("p").wait_for()
        result = self.scan()
        for included in ("Shadow text", "Slotted text", "Frame text", "Below viewport"):
            self.assertIn(included, result["text"])
        self.assertEqual(result["text"].count("Slotted text"), 1)

    def test_unicode_paging_scope_and_invalid_arguments(self):
        self.page.set_content('<main>中文😀abc</main><footer>Outside</footer>')
        first = self.scan(selector="main", limit=3)
        self.assertEqual(first["text"], "中文😀")
        self.assertEqual(first["total_chars"], 6)
        self.assertEqual(first["next_offset"], 3)
        self.assertTrue(first["truncated"])
        last = self.scan(selector="main", offset=3, limit=3)
        self.assertEqual(last["text"], "abc")
        self.assertFalse(last["truncated"])
        self.assertEqual(self.scan(selector="main", offset=100)["returned_chars"], 0)
        self.assertEqual(self.scan(selector="missing")["code"], "element_not_found")
        self.assertEqual(self.scan(selector="[")["code"], "invalid_selector")
        for options in ({"offset": -1}, {"limit": 0}, {"limit": 100001}):
            self.assertEqual(self.scan(**options)["code"], "invalid_arguments")

    def test_hidden_scope_and_inaccessible_frame_report(self):
        self.page.set_content('''<div hidden><p id="scope">Invisible scope</p></div>
          <iframe sandbox srcdoc="<p>Private frame</p>"></iframe>''')
        self.assertEqual(self.scan(selector="#scope")["text"], "")
        result = self.scan()
        self.assertNotIn("Private frame", result["text"])
        self.assertIn("inaccessible_frame", result["warnings"])


class EmbeddedPageTextTests(unittest.IsolatedAsyncioTestCase):
    @unittest.skipUnless(os.environ.get("TESHI_SCAN_TEST_BIN"), "Set TESHI_SCAN_TEST_BIN to a checkout CLI build")
    async def test_cli_scan_reads_real_chromium_and_reports_scope_failures(self):
        from browser_service import EmbeddedSession, handle_embedded_command
        from playwright.async_api import async_playwright
        import websockets

        async with async_playwright() as playwright:
            browser = await playwright.chromium.launch()
            try:
                session = EmbeddedSession()
                session.page = await browser.new_page()
                await session.page.set_content("<main>中文😀abc</main>")

                async def handler(websocket):
                    try:
                        async for message in websocket:
                            response = await handle_embedded_command(session, json.loads(message))
                            await websocket.send(json.dumps(response))
                    except websockets.exceptions.ConnectionClosed:
                        # The CLI closes its one-command transport after receiving a response.
                        pass

                async with websockets.serve(handler, "127.0.0.1", 0) as server:
                    with tempfile.TemporaryDirectory(prefix="teshi-scan-e2e-") as directory:
                        endpoint = Path(directory) / ".teshi/cdp-endpoint.json"
                        endpoint.parent.mkdir()
                        endpoint.write_text(json.dumps({"mode": "embedded", "bridge": "python",
                            "ws_url": f"ws://127.0.0.1:{server.sockets[0].getsockname()[1]}"}), encoding="utf-8")

                        async def run(*arguments):
                            process = await asyncio.create_subprocess_exec(
                                os.environ["TESHI_SCAN_TEST_BIN"], "browser", "scan", *arguments,
                                cwd=directory, env={**os.environ, "TESHI_BROWSER_AUTO_RECONNECT": "0"},
                                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
                            )
                            stdout, stderr = await asyncio.wait_for(process.communicate(), 15)
                            return process.returncode, stdout, stderr

                        code, stdout, stderr = await run("--text-only", "--selector", "main", "--limit", "3")
                        self.assertEqual(code, 0, stderr.decode())
                        self.assertEqual(json.loads(stdout)["text"], "中文😀")
                        code, stdout, stderr = await run("--selector", "main", "--offset", "3", "--limit", "3")
                        self.assertEqual(code, 0, stderr.decode())
                        self.assertEqual(json.loads(stdout)["text"], "abc")
                        code, _, stderr = await run("--selector", "missing")
                        self.assertNotEqual(code, 0)
                        self.assertIn("Text scope did not resolve", stderr.decode())
                        code, _, _ = await run("--limit", "100001")
                        self.assertNotEqual(code, 0)
            finally:
                await browser.close()

    async def test_command_handler_reads_actual_page_and_preserves_request(self):
        from browser_service import EmbeddedSession, handle_embedded_command
        from playwright.async_api import async_playwright

        async with async_playwright() as playwright:
            browser = await playwright.chromium.launch()
            try:
                session = EmbeddedSession()
                session.page = await browser.new_page()
                await session.page.set_content("<title>Example</title><main>中文😀abc</main>")
                response = await handle_embedded_command(session, {
                    "cmd": "get_page_text", "request_id": "scan-test", "selector": "main", "limit": 3,
                })
                self.assertTrue(response["ok"])
                self.assertEqual(response["request_id"], "scan-test")
                self.assertEqual(response["text"], "中文😀")
                self.assertEqual(response["title"], "Example")
                self.assertEqual(response["next_offset"], 3)
            finally:
                await browser.close()


if __name__ == "__main__":
    unittest.main()
