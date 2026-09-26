"""Real Chromium two-Profile acceptance for the P0 browser CLI control loop."""

from __future__ import annotations

import asyncio
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from playwright.async_api import BrowserContext, Worker, async_playwright


REPO_ROOT = Path(__file__).resolve().parents[2]
TESHI_CLI = Path(
    os.environ.get(
        "TESHI_CLI",
        REPO_ROOT / "target" / "debug" / ("teshi.exe" if os.name == "nt" else "teshi"),
    )
)
EXTENSION = REPO_ROOT / "extension" / "teshi-bridge"
DISCOVERY_PORT = int(os.environ.get("TESHI_P0_DISCOVERY_PORT", "17373"))
START_ISOLATED_BROKER = os.environ.get("TESHI_P0_START_ISOLATED_BROKER") == "1"
REDACTED = "<redacted>"
_SECRET_FIELD = re.compile(
    r'(?i)(["\']?(?:lease_token|token|grant_token|access_token)["\']?\s*[:=]\s*["\'])[^"\']*(["\'])'
)
_SECRET_QUERY = re.compile(r"(?i)([?&](?:lease_token|token|grant_token|access_token)=)[^&\s\"]+")


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
        self.stage = "setup"
        self.temp = tempfile.TemporaryDirectory(prefix="teshi-p0-two-profile-")
        self.contexts: list[BrowserContext] = []
        self.broker: subprocess.Popen[str] | None = None
        self.broker_stderr: list[str] = []
        self.broker_stderr_thread: threading.Thread | None = None
        self.project_root = REPO_ROOT
        self.extension_path = EXTENSION
        self.workers: list[Worker] = []
        artifact_dir = os.environ.get("TESHI_P0_ARTIFACT_DIR")
        self.artifact_dir = Path(artifact_dir) if artifact_dir else None
        if self.artifact_dir is not None:
            self.artifact_dir.mkdir(parents=True, exist_ok=True)
            self.diagnostics_path = self.artifact_dir / "two-profile-p0.jsonl"
        else:
            self.diagnostics_path = None
        self.addAsyncCleanup(self._cleanup_browser_profiles)
        if not TESHI_CLI.is_file():
            self.fail(f"built teshi CLI is missing: {TESHI_CLI}")
        if START_ISOLATED_BROKER:
            broker_temp = tempfile.TemporaryDirectory(prefix="teshi-p0-python-broker-")
            self.broker_temp = broker_temp
            self.project_root = Path(broker_temp.name).resolve()
            self.extension_path = Path(self.temp.name) / "teshi-bridge"
            shutil.copytree(EXTENSION, self.extension_path)
            self._rewrite_extension_port()
            self.previous_existing_endpoint_mode = os.environ.get(
                "TESHI_BROWSER_BROKER_USE_EXISTING_ENDPOINT"
            )
            os.environ["TESHI_BROWSER_BROKER_USE_EXISTING_ENDPOINT"] = "1"
            await self._start_isolated_broker()
        self._log(
            "test_start",
            {
                "pid": os.getpid(),
                "cwd": str(REPO_ROOT),
                "project_root": str(self.project_root),
                "cli": str(TESHI_CLI),
                "extension": str(self.extension_path),
                "python": sys.version,
                "platform": platform.platform(),
                "isolated_broker": START_ISOLATED_BROKER,
            },
        )
        self.http = ThreadingHTTPServer(("127.0.0.1", 0), AcceptancePage)
        self.http_thread = threading.Thread(target=self.http.serve_forever, daemon=True)
        self.http_thread.start()
        self._set_stage("discovery and broker preflight")
        if START_ISOLATED_BROKER:
            self.baseline = set()
            self._log(
                "baseline_sessions",
                {"sessions": [], "reason": "isolated broker endpoint is already prepared"},
            )
        else:
            baseline_result = await self.cli("sessions", timeout=20)
            self.baseline = {
                session["identity"]["extension_instance_id"]
                for session in baseline_result["sessions"]
            }
        self._log("runtime_preflight", {
            "discovery_port": DISCOVERY_PORT,
            "port_processes": self._discovery_port_processes(),
            "endpoint": self._endpoint_record(),
            "broker_pid": self.broker.pid if self.broker else None,
            "http_port": self.http.server_address[1],
        })
        self.playwright = await async_playwright().start()
        chromium = Path(self.playwright.chromium.executable_path)
        self._log("runtime_versions", {
            "python": sys.version,
            "rust": self._version_command("rustc", "--version"),
            "node": self._version_command("node", "--version"),
            "playwright": self._version_command(sys.executable, "-m", "playwright", "--version"),
            "chromium_path": str(chromium),
            "chromium_version": self._file_version(chromium),
            "chrome_path": os.environ.get("TESHI_CHROME_PATH", "not_selected_by_test"),
        })
        extension_arg = str(self.extension_path.resolve())
        launch_args = [
            f"--disable-extensions-except={extension_arg}",
            f"--load-extension={extension_arg}",
        ]
        for name in ("profile-a", "profile-b"):
            context = await self.playwright.chromium.launch_persistent_context(
                Path(self.temp.name) / name,
                executable_path=chromium,
                headless=False,
                ignore_default_args=["--disable-extensions"],
                args=[*launch_args, "--window-position=-32000,-32000", "--start-minimized"],
            )
            self.contexts.append(context)
            if context.service_workers:
                self.workers.append(context.service_workers[0])
            else:
                self.workers.append(
                    await context.wait_for_event("serviceworker", timeout=15_000)
                )
        port = self.http.server_address[1]
        self._set_stage("navigate bootstrap pages")
        await asyncio.gather(
            self.contexts[0].pages[0].goto(f"http://127.0.0.1:{port}/bootstrap-a"),
            self.contexts[1].pages[0].goto(f"http://127.0.0.1:{port}/bootstrap-b"),
        )
        await asyncio.gather(
            *(self._log_worker_discovery(index, worker) for index, worker in enumerate(self.workers))
        )
        self._log("profiles_started", {
            "temp_root": self.temp.name,
            "profile_dirs": [str(Path(self.temp.name) / name) for name in ("profile-a", "profile-b")],
            "test_pid": os.getpid(),
            "browser_processes": self._test_profile_processes(),
        })

    def _set_stage(self, stage: str) -> None:
        self.stage = stage
        self._log("stage_start", {"stage": stage})

    def _rewrite_extension_port(self) -> None:
        path = self.extension_path / "background.js"
        text = path.read_text(encoding="utf-8")
        text, replacements = re.subn(
            r"http://127\.0\.0\.1:\d+",
            f"http://127.0.0.1:{DISCOVERY_PORT}",
            text,
        )
        if replacements != 3:
            self.fail(
                f"expected three Broker URLs in copied extension, found {replacements}"
            )
        path.write_text(text, encoding="utf-8")

    def _drain_broker_stderr(self, stream: object) -> None:
        for line in stream:  # type: ignore[union-attr]
            self.broker_stderr.append(str(line).rstrip())

    async def _start_isolated_broker(self) -> None:
        args = [
            sys.executable,
            str(REPO_ROOT / "resources" / "browser_service.py"),
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--mode",
            "chrome",
            "--discovery-port",
            str(DISCOVERY_PORT),
            "--project-root",
            str(self.project_root),
        ]
        self.broker = subprocess.Popen(
            args,
            cwd=REPO_ROOT,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        assert self.broker.stdout is not None
        assert self.broker.stderr is not None
        self.broker_stderr_thread = threading.Thread(
            target=self._drain_broker_stderr,
            args=(self.broker.stderr,),
            daemon=True,
        )
        self.broker_stderr_thread.start()
        ready_line = await asyncio.wait_for(
            asyncio.to_thread(self.broker.stdout.readline), timeout=15
        )
        if not ready_line.strip().isdigit():
            self._log(
                "broker_start_failed",
                {
                    "pid": self.broker.pid,
                    "returncode": self.broker.poll(),
                    "stdout": ready_line,
                    "stderr": self.broker_stderr[-40:],
                },
            )
            self.fail(
                "isolated Python Broker did not announce its WebSocket port: "
                f"stdout={ready_line!r}, stderr={self.broker_stderr[-10:]}"
            )
        self._log(
            "broker_started",
            {
                "pid": self.broker.pid,
                "discovery_port": DISCOVERY_PORT,
                "websocket_port": int(ready_line.strip()),
            },
        )

    async def _log_worker_discovery(self, index: int, worker: Worker) -> None:
        try:
            diagnostics = await worker.evaluate(
                """async () => {
                    const response = await fetch(DISCOVERY_URL, discoveryRequestOptions());
                    const body = await response.text();
                    let info = {};
                    try { info = body ? JSON.parse(body) : {}; } catch {}
                    return {
                        origin: globalThis.location?.origin || "",
                        status: response.status,
                        bodyLength: body.length,
                        keys: Object.keys(info).sort(),
                        hasTokenizedWs: String(info.ws_url || "").includes("token="),
                        hasTokenizedFrameWs: String(info.extension_frame_ws_url || "").includes("token="),
                        bridge: info.bridge || "",
                        brokerFeatures: info.broker_features || [],
                        cache: {
                            projectRoot: cachedProjectRoot,
                            projectNeutral: cachedBrokerProjectNeutral,
                            tokenCached: Boolean(cachedBrokerToken),
                            frameWs: Boolean(extensionFrameWsUrl),
                            lastBridgeStatus,
                        },
                    };
                }"""
            )
        except Exception as error:  # noqa: BLE001
            diagnostics = {"worker_error": str(error)}
        self._log("extension_discovery_probe", {"profile_index": index, **diagnostics})

    @staticmethod
    def _redact(value: object) -> object:
        if isinstance(value, dict):
            return {key: BrowserTwoProfileP0Tests._redact(item) for key, item in value.items()}
        if isinstance(value, list):
            return [BrowserTwoProfileP0Tests._redact(item) for item in value]
        if not isinstance(value, str):
            return value
        value = _SECRET_FIELD.sub(rf"\1{REDACTED}\2", value)
        return _SECRET_QUERY.sub(rf"\1{REDACTED}", value)

    def _log(self, event: str, payload: dict) -> None:
        if self.diagnostics_path is None:
            return
        record = {
            "time": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            "monotonic": round(time.monotonic(), 3),
            "stage": getattr(self, "stage", "setup"),
            "event": event,
            "payload": self._redact(payload),
        }
        with self.diagnostics_path.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(record, ensure_ascii=False, sort_keys=True) + "\n")

    @staticmethod
    def _version_command(*command: str) -> str:
        try:
            result = subprocess.run(
                list(command), capture_output=True, text=True, timeout=10, check=False
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            return f"unavailable: {error}"
        return (result.stdout or result.stderr).strip()

    @staticmethod
    def _file_version(path: Path) -> str:
        if os.name != "nt":
            return "not_available_on_this_platform"
        escaped = str(path).replace("'", "''")
        return BrowserTwoProfileP0Tests._version_command(
            "powershell",
            "-NoProfile",
            "-Command",
            f"(Get-Item -LiteralPath '{escaped}').VersionInfo.FileVersion",
        )

    def _endpoint_record(self) -> dict:
        path = self.project_root / ".teshi" / "cdp-endpoint.json"
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            return {"path": str(path), "error": str(error)}
        return {
            "path": str(path),
            "mode": data.get("mode"),
            "bridge": data.get("bridge"),
            "broker_pid": data.get("broker_pid"),
            "broker_start_id": data.get("broker_start_id"),
            "protocol_version": data.get("protocol_version"),
            "schema_version": data.get("schema_version"),
            "broker_features": data.get("broker_features", []),
            "extension_connected": data.get("extension_connected"),
        }

    @staticmethod
    def _discovery_port_processes() -> list[dict[str, str]]:
        if os.name != "nt":
            return []
        result = subprocess.run(
            ["netstat", "-ano", "-p", "tcp"],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        processes = []
        for line in result.stdout.splitlines():
            fields = line.split()
            if len(fields) >= 5 and fields[1].endswith(f":{DISCOVERY_PORT}"):
                pid = fields[-1]
                processes.append({"local": fields[1], "state": fields[3], "pid": pid})
        return processes

    def _test_profile_processes(self) -> list[dict[str, str]]:
        if os.name != "nt":
            return [{"pid": str(os.getpid()), "name": "test-process"}]
        result = subprocess.run(
            ["powershell", "-NoProfile", "-Command", "Get-CimInstance Win32_Process | Select-Object Name,ProcessId,CommandLine | ConvertTo-Json -Compress"],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        try:
            entries = json.loads(result.stdout) if result.stdout.strip() else []
        except json.JSONDecodeError:
            return [{"pid": str(os.getpid()), "name": "test-process"}]
        if isinstance(entries, dict):
            entries = [entries]
        temp = getattr(self, "temp", None)
        marker = temp.name if temp is not None else ""
        return [
            {"pid": str(item.get("ProcessId")), "name": item.get("Name", "")}
            for item in entries
            if marker and marker in str(item.get("CommandLine", ""))
        ]

    async def _cleanup_browser_profiles(self) -> None:
        self._set_stage("cleanup test-created browser processes")
        self._log("cleanup_start", {
            "test_pid": os.getpid(),
            "browser_processes": self._test_profile_processes(),
            "profile_dirs": [str(context) for context in getattr(self, "contexts", [])],
        })
        for context in reversed(getattr(self, "contexts", [])):
            await context.close()
        self._log("browser_contexts_closed", {
            "test_pid": os.getpid(),
            "browser_processes": self._test_profile_processes(),
        })
        if getattr(self, "playwright", None) is not None:
            await self.playwright.stop()
        if getattr(self, "http", None) is not None:
            self.http.shutdown()
            self.http.server_close()
        if getattr(self, "http_thread", None) is not None:
            self.http_thread.join(timeout=2)
        if getattr(self, "temp", None) is not None:
            self.temp.cleanup()
        broker = self.broker
        self.broker = None
        if broker is not None:
            if broker.poll() is None:
                broker.terminate()
                try:
                    await asyncio.to_thread(broker.wait, 10)
                except subprocess.TimeoutExpired:
                    broker.kill()
                    await asyncio.to_thread(broker.wait, 5)
            if self.broker_stderr_thread is not None:
                self.broker_stderr_thread.join(timeout=2)
            if broker.stdout is not None:
                broker.stdout.close()
            if broker.stderr is not None:
                broker.stderr.close()
            self._log(
                "broker_stopped",
                {
                    "pid": broker.pid,
                    "returncode": broker.returncode,
                    "stderr": self.broker_stderr[-40:],
                },
            )
        if getattr(self, "broker_temp", None) is not None:
            self.broker_temp.cleanup()
        if hasattr(self, "previous_existing_endpoint_mode"):
            previous = self.previous_existing_endpoint_mode
            if previous is None:
                os.environ.pop("TESHI_BROWSER_BROKER_USE_EXISTING_ENDPOINT", None)
            else:
                os.environ["TESHI_BROWSER_BROKER_USE_EXISTING_ENDPOINT"] = previous
        self._log("cleanup_complete", {"test_pid": os.getpid()})

    async def cli(self, *args: str, timeout: float = 30) -> dict:
        redacted_args = self._redacted_args(args)
        self._log("cli_start", {"args": redacted_args, "timeout_seconds": timeout})
        def invoke() -> subprocess.CompletedProcess[str]:
            return subprocess.run(
                [str(TESHI_CLI), "browser", *args],
                cwd=self.project_root,
                capture_output=True,
                text=True,
                timeout=timeout,
                check=False,
            )

        try:
            result = await asyncio.to_thread(invoke)
        except subprocess.TimeoutExpired as error:
            self._log("cli_timeout", {
                "args": redacted_args,
                "stdout": error.stdout or "",
                "stderr": error.stderr or "",
            })
            self.fail(f"CLI timed out during {self.stage}: {' '.join(redacted_args)}")
        self._log("cli_end", {
            "args": redacted_args,
            "returncode": result.returncode,
            "stdout": result.stdout,
            "stderr": result.stderr,
        })
        self.assertEqual(
            result.returncode,
            0,
            f"CLI failed during {self.stage}: {' '.join(redacted_args)}\nstdout={self._redact(result.stdout)}\nstderr={self._redact(result.stderr)}",
        )
        try:
            return json.loads(result.stdout)
        except json.JSONDecodeError as error:
            self.fail(f"CLI returned invalid JSON during {self.stage}: {error}")

    @staticmethod
    def _redacted_args(args: tuple[str, ...]) -> list[str]:
        redacted = []
        redact_next = False
        for arg in args:
            if redact_next:
                redacted.append(REDACTED)
                redact_next = False
            else:
                redacted.append(arg)
                if arg in {"--lease-token", "--token", "--grant-token"}:
                    redact_next = True
        return redacted

    async def wait_for_new_profiles(self, baseline: set[str]) -> list[dict]:
        self._set_stage("wait for two extension heartbeats and ready sessions")
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
                self._log("profiles_ready", {
                    "profiles": [session["identity"]["extension_instance_id"] for session in new_sessions],
                })
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
        self._log("target_wait_start", {"target": target})
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
                self._log("target_ready", {"target": target})
                return
            await asyncio.sleep(0.1)
        self.fail(f"new target was not published by heartbeat: {target}")

    async def wait_for_target_url(
        self, target: dict[str, str], expected_url: str
    ) -> None:
        """Wait until the Broker heartbeat publishes the completed navigation."""
        self._log(
            "target_url_wait_start",
            {"target": target, "expected_url": expected_url},
        )
        deadline = asyncio.get_running_loop().time() + 10
        consecutive_matches = 0
        while asyncio.get_running_loop().time() < deadline:
            tabs = await self.cli(
                "tabs", "--session", target["session"], timeout=20
            )
            observed = next(
                (
                    tab
                    for window in tabs["windows"]
                    for tab in window["tabs"]
                    if str(tab["id"]) == str(target["tab"])
                ),
                None,
            )
            observed_url = observed.get("url", "") if observed else ""
            if observed_url == expected_url:
                consecutive_matches += 1
                if consecutive_matches >= 2:
                    self._log(
                        "target_url_ready",
                        {
                            "target": target,
                            "url": observed_url,
                            "consecutive_matches": consecutive_matches,
                        },
                    )
                    return
            else:
                consecutive_matches = 0
            await asyncio.sleep(0.1)
        self.fail(
            f"Broker heartbeat did not publish a stable URL for {target}: "
            f"expected {expected_url!r}, observed {observed_url!r}"
        )

    async def test_two_profiles_execute_concurrently_without_cross_routing(self) -> None:
        self._set_stage("register two isolated Profiles")
        sessions = await self.wait_for_new_profiles(self.baseline)
        sessions.sort(key=lambda item: item["identity"]["extension_instance_id"])
        targets = [self.active_target(session) for session in sessions]

        self._set_stage("acquire independent leases")
        await asyncio.gather(
            self.cli("profile-label", "set", "--session", targets[0]["session"], "--label", f"P0 Agent A {targets[0]['session'][:8]}"),
            self.cli("profile-label", "set", "--session", targets[1]["session"], "--label", f"P0 Agent B {targets[1]['session'][:8]}"),
        )
        leases = await asyncio.gather(
            self.cli("lease", "acquire", "--session", targets[0]["session"], "--owner", "p0-agent-a"),
            self.cli("lease", "acquire", "--session", targets[1]["session"], "--owner", "p0-agent-b"),
        )
        tokens = [lease["lease"]["lease_token"] for lease in leases]
        self._log("leases_acquired", {
            "owners": [lease["lease"]["owner_label"] for lease in leases],
            "profiles": [target["session"] for target in targets],
        })

        try:
            port = self.http.server_address[1]
            self._set_stage("navigate both Profiles")
            await asyncio.gather(
                self.cli("navigate", f"http://127.0.0.1:{port}/a", *self.target_args(targets[0], tokens[0])),
                self.cli("navigate", f"http://127.0.0.1:{port}/b", *self.target_args(targets[1], tokens[1])),
            )
            self._set_stage("wait for navigation heartbeats")
            await asyncio.gather(
                self.wait_for_target_url(
                    targets[0], f"http://127.0.0.1:{port}/a"
                ),
                self.wait_for_target_url(
                    targets[1], f"http://127.0.0.1:{port}/b"
                ),
            )
            self._set_stage("snapshot and resolve profile-local references")
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

            self._set_stage("click and pointer-click concurrently")
            await asyncio.gather(
                self.cli("execute", "--reference", refs[0], "--action", "click", "--wait-text", "clicked-a", *self.target_args(targets[0], tokens[0])),
                self.cli("execute", "--reference", refs[1], "--action", "pointer_click", "--wait-text", "clicked-b", *self.target_args(targets[1], tokens[1])),
            )
            pages = [context.pages[0] for context in self.contexts]
            observed = sorted([await page.locator("#status").text_content() for page in pages])
            self._log("dom_side_effects", {"status_values": observed})
            self.assertEqual(observed, ["clicked-a", "clicked-b"])

            self._set_stage("open, observe, and close independent tabs")
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
            self._log("tab_lifecycle_complete", {"new_targets": new_targets})
        finally:
            self._set_stage("release both leases and verify cleanup")
            await asyncio.gather(
                self.cli("lease", "release", "--session", targets[0]["session"], "--lease-token", tokens[0]),
                self.cli("lease", "release", "--session", targets[1]["session"], "--lease-token", tokens[1]),
            )
            sessions_after_release = (await self.cli("sessions"))["sessions"]
            by_id = {
                session["identity"]["extension_instance_id"]: session
                for session in sessions_after_release
            }
            for target in targets:
                self.assertIsNone(by_id[target["session"]].get("lease"))
            self._log("leases_released", {
                "profiles": [target["session"] for target in targets],
                "lease_fields_clear": True,
            })


if __name__ == "__main__":
    unittest.main()
