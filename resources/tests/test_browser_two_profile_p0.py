"""Real Chromium two-Profile acceptance for the P0 browser CLI control loop."""

from __future__ import annotations

import asyncio
import json
import os
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from playwright.async_api import BrowserContext, async_playwright


REPO_ROOT = Path(__file__).resolve().parents[2]
TESHI_CLI = Path(
    os.environ.get(
        "TESHI_CLI",
        REPO_ROOT / "target" / "debug" / ("teshi.exe" if os.name == "nt" else "teshi"),
    )
)
EXTENSION = REPO_ROOT / "extension" / "teshi-bridge"


class AcceptancePage(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 - stdlib callback name
        profile = self.path.strip("/") or "unknown"
        body = f"""<!doctype html><meta charset=utf-8>
        <title>Profile {profile}</title>
        <button id=action onclick="document.querySelector('#status').textContent='clicked-{profile}'">Run {profile}</button>
        <input id=upload type=file onchange="document.querySelector('#upload-status').textContent='uploaded-' + this.files[0].name">
        <div id=status>idle-{profile}</div>
        <div id=upload-status>upload-idle</div>""".encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: object) -> None:
        return


class BrowserTwoProfileP0Tests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        if not TESHI_CLI.is_file():
            self.fail(f"built teshi CLI is missing: {TESHI_CLI}")
        self.temp = tempfile.TemporaryDirectory(prefix="teshi-p0-two-profile-")
        self.http = ThreadingHTTPServer(("127.0.0.1", 0), AcceptancePage)
        self.http_thread = threading.Thread(target=self.http.serve_forever, daemon=True)
        self.http_thread.start()
        baseline_result = subprocess.run(
            [str(TESHI_CLI), "browser", "sessions"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            timeout=20,
            check=True,
        )
        self.baseline = {
            session["identity"]["extension_instance_id"]
            for session in json.loads(baseline_result.stdout)["sessions"]
        }
        self.playwright = await async_playwright().start()
        chromium = Path(self.playwright.chromium.executable_path)
        extension_arg = str(EXTENSION.resolve())
        launch_args = [
            f"--disable-extensions-except={extension_arg}",
            f"--load-extension={extension_arg}",
        ]
        self.contexts: list[BrowserContext] = []
        for name in ("profile-a", "profile-b"):
            context = await self.playwright.chromium.launch_persistent_context(
                Path(self.temp.name) / name,
                executable_path=chromium,
                headless=False,
                ignore_default_args=["--disable-extensions"],
                args=[*launch_args, "--window-position=-32000,-32000", "--start-minimized"],
            )
            self.contexts.append(context)
        port = self.http.server_address[1]
        await asyncio.gather(
            self.contexts[0].pages[0].goto(f"http://127.0.0.1:{port}/bootstrap-a"),
            self.contexts[1].pages[0].goto(f"http://127.0.0.1:{port}/bootstrap-b"),
        )

    async def asyncTearDown(self) -> None:
        for context in reversed(getattr(self, "contexts", [])):
            await context.close()
        if getattr(self, "playwright", None) is not None:
            await self.playwright.stop()
        if getattr(self, "http", None) is not None:
            self.http.shutdown()
            self.http.server_close()
        if getattr(self, "http_thread", None) is not None:
            self.http_thread.join(timeout=2)
        if getattr(self, "temp", None) is not None:
            self.temp.cleanup()

    async def cli(self, *args: str, timeout: float = 30) -> dict:
        def invoke() -> subprocess.CompletedProcess[str]:
            return subprocess.run(
                [str(TESHI_CLI), "browser", *args],
                cwd=REPO_ROOT,
                capture_output=True,
                text=True,
                timeout=timeout,
                check=False,
            )

        result = await asyncio.to_thread(invoke)
        self.assertEqual(
            result.returncode,
            0,
            f"CLI failed: {' '.join(args)}\nstdout={result.stdout}\nstderr={result.stderr}",
        )
        return json.loads(result.stdout)

    async def wait_for_new_profiles(self, baseline: set[str]) -> list[dict]:
        deadline = asyncio.get_running_loop().time() + 20
        while asyncio.get_running_loop().time() < deadline:
            sessions = (await self.cli("sessions"))["sessions"]
            new_sessions = []
            for session in sessions:
                has_debuggable_active_tab = any(
                    tab.get("active") and tab.get("debuggable")
                    for window in session["windows"]
                    for tab in window["tabs"]
                )
                if (
                    session["identity"]["extension_instance_id"] not in baseline
                    and session["health"] == "ready"
                    and has_debuggable_active_tab
                ):
                    new_sessions.append(session)
            if len(new_sessions) == 2:
                return new_sessions
            await asyncio.sleep(0.25)
        self.fail("two temporary extension Profiles did not register with the broker")

    @staticmethod
    def active_target(session: dict) -> dict[str, str]:
        identity = session["identity"]["extension_instance_id"]
        for window in session["windows"]:
            for tab in window["tabs"]:
                if tab.get("active"):
                    return {
                        "session": identity,
                        "window": str(window["id"]),
                        "tab": str(tab["id"]),
                    }
        raise AssertionError(f"session has no active tab: {identity}")

    @staticmethod
    def target_args(target: dict[str, str], lease: str) -> list[str]:
        return [
            "--session",
            target["session"],
            "--window",
            target["window"],
            "--tab",
            target["tab"],
            "--lease-token",
            lease,
        ]

    async def wait_for_target(self, target: dict) -> None:
        deadline = asyncio.get_running_loop().time() + 10
        while asyncio.get_running_loop().time() < deadline:
            tabs = await self.cli(
                "tabs", "--session", target["extension_instance_id"]
            )
            if any(
                tab["id"] == target["tab_id"]
                for window in tabs["windows"]
                for tab in window["tabs"]
            ):
                return
            await asyncio.sleep(0.1)
        self.fail(f"new target was not published by heartbeat: {target}")

    async def test_two_profiles_execute_concurrently_without_cross_routing(self) -> None:
        sessions = await self.wait_for_new_profiles(self.baseline)
        sessions.sort(key=lambda item: item["identity"]["extension_instance_id"])
        targets = [self.active_target(session) for session in sessions]

        await asyncio.gather(
            self.cli("profile-label", "set", "--session", targets[0]["session"], "--label", f"P0 Agent A {targets[0]['session'][:8]}"),
            self.cli("profile-label", "set", "--session", targets[1]["session"], "--label", f"P0 Agent B {targets[1]['session'][:8]}"),
        )
        leases = await asyncio.gather(
            self.cli("lease", "acquire", "--session", targets[0]["session"], "--owner", "p0-agent-a"),
            self.cli("lease", "acquire", "--session", targets[1]["session"], "--owner", "p0-agent-b"),
        )
        tokens = [lease["lease"]["lease_token"] for lease in leases]

        try:
            port = self.http.server_address[1]
            await asyncio.gather(
                self.cli("navigate", f"http://127.0.0.1:{port}/a", *self.target_args(targets[0], tokens[0])),
                self.cli("navigate", f"http://127.0.0.1:{port}/b", *self.target_args(targets[1], tokens[1])),
            )
            snapshots = await asyncio.gather(
                self.cli("snapshot", *self.target_args(targets[0], tokens[0])),
                self.cli("snapshot", *self.target_args(targets[1], tokens[1])),
            )
            refs = []
            for snapshot in snapshots:
                button = next(
                    element
                    for element in snapshot["interactive_elements"]
                    if element.get("tag") == "button" or element.get("role") == "button"
                )
                refs.append(button["ref"])
            self.assertEqual(refs, ["@e1", "@e1"], "aliases should be profile-local")

            await asyncio.gather(
                self.cli("execute", "--reference", refs[0], "--action", "click", "--wait-text", "clicked-a", *self.target_args(targets[0], tokens[0])),
                self.cli("execute", "--reference", refs[1], "--action", "pointer_click", "--wait-text", "clicked-b", *self.target_args(targets[1], tokens[1])),
            )
            pages = [context.pages[0] for context in self.contexts]
            observed = sorted([await page.locator("#status").text_content() for page in pages])
            self.assertEqual(observed, ["clicked-a", "clicked-b"])

            opened = await asyncio.gather(
                self.cli("tab", "open", f"http://127.0.0.1:{port}/a-new", "--active", *self.target_args(targets[0], tokens[0])),
                self.cli("tab", "open", f"http://127.0.0.1:{port}/b-new", "--active", *self.target_args(targets[1], tokens[1])),
            )
            new_targets = [item["new_target"] for item in opened]
            self.assertNotEqual(new_targets[0]["extension_instance_id"], new_targets[1]["extension_instance_id"])
            await asyncio.gather(*(self.wait_for_target(target) for target in new_targets))
            await asyncio.gather(
                self.cli("tab", "close", *self.target_args({"session": new_targets[0]["extension_instance_id"], "window": str(new_targets[0]["window_id"]), "tab": str(new_targets[0]["tab_id"])}, tokens[0])),
                self.cli("tab", "close", *self.target_args({"session": new_targets[1]["extension_instance_id"], "window": str(new_targets[1]["window_id"]), "tab": str(new_targets[1]["tab_id"])}, tokens[1])),
            )
        finally:
            await asyncio.gather(
                self.cli("lease", "release", "--session", targets[0]["session"], "--lease-token", tokens[0]),
                self.cli("lease", "release", "--session", targets[1]["session"], "--lease-token", tokens[1]),
            )


if __name__ == "__main__":
    unittest.main()
