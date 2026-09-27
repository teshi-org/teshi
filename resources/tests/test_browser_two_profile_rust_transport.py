"""Real Chromium two-Profile Navigation/Snapshot acceptance for the Rust Broker."""

from __future__ import annotations

import asyncio
import contextlib
import json
import os
import platform
import queue
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, urlencode, urlsplit, urlunsplit
from urllib.request import Request, urlopen

import websockets
from playwright.async_api import BrowserContext, Worker, async_playwright


REPO_ROOT = Path(__file__).resolve().parents[2]
TESHI_CLI = Path(
    os.environ.get(
        "TESHI_CLI",
        REPO_ROOT / "target" / "debug" / ("teshi.exe" if os.name == "nt" else "teshi"),
    )
)
EXTENSION = REPO_ROOT / "extension" / "teshi-bridge"
SECRET_QUERY = re.compile(r"(?i)([?&]token=)[^&\s\"]+")


class RustTransportPage(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 - stdlib callback name
        path = self.path.split("?", 1)[0]
        profile = path.strip("/") or "unknown"
        if path.endswith("/locator-frame"):
            body = (
                "<!doctype html><meta charset=utf-8><title>Locator frame</title>"
                "<button id=\"frame-submit\" data-testid=\"frame-submit\" "
                "onclick=\"document.querySelector('#frame-click-count').textContent='1'\">"
                "Frame submit</button><span id=\"frame-click-count\">0</span>"
            ).encode()
        elif path.endswith("/locator-context"):
            port = self.server.server_address[1]
            frame_url = f"http://127.0.0.1:{port}/locator-frame"
            body = (
                "<!doctype html><meta charset=utf-8><title>Locator context</title>"
                "<label for=\"context-email\">Email address</label>"
                "<input id=\"context-email\" name=\"email\" placeholder=\"name@example.test\" "
                "data-qa=\"context-email\">"
                "<input class=\"css-only-target\" onclick=\"document.querySelector('#css-click-count').textContent='1'\">"
                "<span id=\"css-click-count\">0</span>"
                f"<iframe name=\"checkout-frame\" src=\"{frame_url}\"></iframe>"
                f"<iframe name=\"cross-origin-frame\" src=\"http://localhost:{port}/locator-frame\"></iframe>"
                "<div id=\"shadow-widget\"></div>"
                "<script>"
                "const host=document.querySelector('#shadow-widget');"
                "const root=host.attachShadow({mode:'open'});"
                "root.innerHTML=\"<button id='shadow-submit' data-testid='shadow-submit' "
                "onclick=\\\"this.nextElementSibling.textContent='1'\\\">Shadow submit</button>"
                "<span id='shadow-click-count'>0</span>\";"
                "</script>"
            ).encode()
        else:
            body = (
                f"<!doctype html><meta charset=utf-8><title>Rust {profile}</title>"
                f"<main data-profile=\"{profile}\"><h1>Rust {profile}</h1>"
                f"<button id=\"snapshot-action\">Snapshot {profile}</button>"
                f"<button id=\"dom-click\" onclick=\"document.querySelector('#dom-click-count').textContent=String(Number(document.querySelector('#dom-click-count').textContent)+1)\">DOM click {profile}</button>"
                f"<button id=\"pointer-click\" onpointerdown=\"document.querySelector('#pointerdown-count').textContent=String(Number(document.querySelector('#pointerdown-count').textContent)+1)\" onclick=\"document.querySelector('#pointer-click-count').textContent=String(Number(document.querySelector('#pointer-click-count').textContent)+1)\">Pointer click {profile}</button>"
                f"<button id=\"test-id-click\" data-testid=\"test-id-click\" onclick=\"document.querySelector('#test-id-click-count').textContent=String(Number(document.querySelector('#test-id-click-count').textContent)+1)\">Test ID click {profile}</button>"
                f"<button id=\"role-click\" aria-label=\"Role click\" onclick=\"document.querySelector('#role-click-count').textContent=String(Number(document.querySelector('#role-click-count').textContent)+1)\">Role click {profile}</button>"
                f"<button data-testid=\"ambiguous-click\">Ambiguous one {profile}</button>"
                f"<button data-testid=\"ambiguous-click\">Ambiguous two {profile}</button>"
                f"<button id=\"hidden-action\" style=\"display:none\" onclick=\"document.querySelector('#negative-click-count').textContent=String(Number(document.querySelector('#negative-click-count').textContent)+1)\">Hidden action {profile}</button>"
                f"<button id=\"disabled-action\" disabled onclick=\"document.querySelector('#negative-click-count').textContent=String(Number(document.querySelector('#negative-click-count').textContent)+1)\">Disabled action {profile}</button>"
                f"<button id=\"timeout-action\" onclick=\"document.querySelector('#timeout-click-count').textContent=String(Number(document.querySelector('#timeout-click-count').textContent)+1)\">Timeout action {profile}</button>"
                f"<div id=\"assert-target\">Expected {profile}</div>"
                f"<div id=\"dom-click-count\">0</div>"
                f"<div id=\"pointer-click-count\">0</div>"
                f"<div id=\"pointerdown-count\">0</div></main>"
                f"<div id=\"test-id-click-count\">0</div>"
                f"<div id=\"role-click-count\">0</div>"
                f"<div id=\"negative-click-count\">0</div>"
                f"<div id=\"timeout-click-count\">0</div>"
            ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_OPTIONS(self) -> None:  # noqa: N802 - stdlib callback name
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")
        self.end_headers()

    def do_POST(self) -> None:  # noqa: N802 - stdlib callback name
        try:
            length = min(int(self.headers.get("Content-Length", "0")), 8 * 1024 * 1024)
        except ValueError:
            length = 0
        if length:
            self.rfile.read(length)
        body = ("rust-network-response-" + ("r" * 32_768)).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: object) -> None:
        return


class BrowserTwoProfileRustTransportTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.stage = "setup"
        self.artifact_dir = (
            Path(os.environ["TESHI_RUST_TRANSPORT_ARTIFACT_DIR"])
            if os.environ.get("TESHI_RUST_TRANSPORT_ARTIFACT_DIR")
            else None
        )
        if self.artifact_dir is not None:
            self.artifact_dir.mkdir(parents=True, exist_ok=True)
            self.diagnostics_path = self.artifact_dir / "two-profile-rust-transport.jsonl"
        else:
            self.diagnostics_path = None
        self.temp = tempfile.TemporaryDirectory(prefix="teshi-rust-transport-")
        self.temp_root = Path(self.temp.name)
        self.extension_copy = self.temp_root / "teshi-bridge"
        shutil.copytree(EXTENSION, self.extension_copy)
        self.contexts: dict[str, BrowserContext] = {}
        self.workers: dict[str, Worker] = {}
        self._pending_worker_evaluations: set[asyncio.Task] = set()
        self.broker: subprocess.Popen[str] | None = None
        self.broker_stderr: list[str] = []
        self.broker_stderr_thread: threading.Thread | None = None
        self.broker_stdout_lines: queue.Queue[str] = queue.Queue()
        self.broker_stdout_thread: threading.Thread | None = None
        self.http = ThreadingHTTPServer(("127.0.0.1", 0), RustTransportPage)
        self.http_thread = threading.Thread(target=self.http.serve_forever, daemon=True)
        self.http_thread.start()
        self._log(
            "test_start",
            {
                "pid": os.getpid(),
                "cwd": str(REPO_ROOT),
                "cli": str(TESHI_CLI),
                "platform": platform.platform(),
                "python": sys.version,
                "http_port": self.http.server_address[1],
            },
        )
        self.addAsyncCleanup(self._cleanup)
        if not TESHI_CLI.is_file():
            self.fail(f"built teshi CLI is missing: {TESHI_CLI}")

        self.playwright = await async_playwright().start()
        self.chromium_path = Path(self.playwright.chromium.executable_path)
        self.discovery_port = self._unused_port()
        self._rewrite_extension_port(self.discovery_port)
        self._log(
            "runtime_versions",
            {
                "rust": self._version("rustc", "--version"),
                "node": self._version("node", "--version"),
                "playwright": self._version(
                    sys.executable, "-m", "playwright", "--version"
                ),
                "chromium": self._version(str(self.chromium_path), "--version"),
                "chromium_path": str(self.chromium_path),
            },
        )

        self._set_stage("probe extension identity")
        probe_context = await self._launch_context("probe", keep=False)
        try:
            probe_worker = await self._wait_worker(probe_context)
            self.extension_origin = self._worker_origin(probe_worker)
            self._log(
                "browser_engine_version",
                {"chromium": probe_context.browser.version},
            )
            self._log(
                "extension_identity",
                {"origin": self.extension_origin, "probe_pid": os.getpid()},
            )
        finally:
            await probe_context.close()

        self._set_stage("start isolated Rust transport broker")
        await self._start_broker()
        self._set_stage("launch two Chromium Profiles")
        await self._launch_context("profile-a")
        await self._launch_context("profile-b")
        port = self.http.server_address[1]
        await asyncio.wait_for(
            self.contexts["profile-a"].pages[0].goto(
                f"http://127.0.0.1:{port}/rust-a", wait_until="domcontentloaded"
            ),
            timeout=20,
        )
        await asyncio.wait_for(
            self.contexts["profile-b"].pages[0].goto(
                f"http://127.0.0.1:{port}/rust-b", wait_until="domcontentloaded"
            ),
            timeout=20,
        )
        await asyncio.gather(
            self._force_stream("profile-a"),
            self._force_stream("profile-b"),
        )
        self._log(
            "profiles_started",
            {
                "temp_root": str(self.temp_root),
                "profile_dirs": [
                    str(self.temp_root / "profile-a"),
                    str(self.temp_root / "profile-b"),
                ],
                "test_pid": os.getpid(),
                "broker_pid": self.broker.pid if self.broker else None,
            },
        )

    async def _cleanup(self) -> None:
        self._set_stage("cleanup test-created Rust transport resources")
        for name in list(self.contexts):
            context = self.contexts.pop(name)
            try:
                await asyncio.wait_for(context.close(), timeout=10)
            except Exception as error:  # noqa: BLE001
                self._log("context_close_error", {"profile": name, "error": str(error)})
        await self._drain_worker_evaluations()
        await self._stop_broker()
        if hasattr(self, "playwright"):
            try:
                await asyncio.wait_for(self.playwright.stop(), timeout=10)
            except Exception as error:  # noqa: BLE001
                self._log("playwright_stop_error", {"error": str(error)})
        if hasattr(self, "http"):
            try:
                await asyncio.wait_for(asyncio.to_thread(self.http.shutdown), timeout=5)
            except asyncio.TimeoutError:
                self._log("http_shutdown_timeout", {})
            self.http.server_close()
            self.http_thread.join(timeout=3)
        self._log(
            "cleanup_complete",
            {
                "test_pid": os.getpid(),
                "broker_pid": self.broker.pid if self.broker else None,
            },
        )
        self.temp.cleanup()

    async def _drain_worker_evaluations(self) -> None:
        """Consume Playwright callbacks left behind by bounded worker probes."""
        pending = list(self._pending_worker_evaluations)
        for task in pending:
            if not task.done():
                try:
                    await asyncio.wait_for(asyncio.shield(task), timeout=3)
                except Exception:  # noqa: BLE001 - cleanup must continue
                    task.cancel()
            with contextlib.suppress(BaseException):
                await task
            self._pending_worker_evaluations.discard(task)

    def _consume_worker_evaluation(self, task: asyncio.Task) -> None:
        self._pending_worker_evaluations.discard(task)
        with contextlib.suppress(BaseException):
            task.result()

    async def _evaluate_worker(
        self, worker: Worker, expression: str, *, timeout: float
    ) -> object:
        """Bound a worker RPC without abandoning its underlying Playwright Future."""
        task = asyncio.create_task(worker.evaluate(expression))
        try:
            return await asyncio.wait_for(asyncio.shield(task), timeout=timeout)
        except asyncio.TimeoutError:
            self._pending_worker_evaluations.add(task)
            task.add_done_callback(self._consume_worker_evaluation)
            raise
        except BaseException:
            with contextlib.suppress(BaseException):
                await task
            raise

    async def _start_broker(self) -> None:
        state_dir = self.temp_root / "rust-user-state"
        args = [
            str(TESHI_CLI),
            "--browser-broker-internal",
            "--state-dir",
            str(state_dir),
            "--trusted-extension-origin",
            self.extension_origin,
            "--enable-p0-control",
            "--enable-p1-observability",
            "--discovery-port",
            str(self.discovery_port),
        ]
        self.broker = subprocess.Popen(
            args,
            cwd=REPO_ROOT,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        assert self.broker.stdout is not None
        assert self.broker.stderr is not None
        self.broker_stderr_thread = threading.Thread(
            target=self._drain_broker_stderr,
            args=(self.broker.stderr,),
            daemon=True,
        )
        self.broker_stderr_thread.start()
        self.broker_stdout_thread = threading.Thread(
            target=self._drain_broker_stdout,
            args=(self.broker.stdout,),
            daemon=True,
        )
        self.broker_stdout_thread.start()
        ready_line = await self._wait_broker_stdout_line()
        if not ready_line.startswith("BROWSER_BROKER_READY "):
            raise AssertionError(f"Rust Broker did not announce readiness: {ready_line!r}")
        self.ready_endpoint = json.loads(
            ready_line.removeprefix("BROWSER_BROKER_READY ").strip()
        )
        self._log(
            "broker_ready",
            {
                "pid": self.broker.pid,
                "broker_start_id": self.ready_endpoint.get("broker_start_id"),
                "protocol_version": self.ready_endpoint.get("protocol_version"),
                "schema_version": self.ready_endpoint.get("schema_version"),
                "broker_features": self.ready_endpoint.get("broker_features"),
                "discovery_port": self.discovery_port,
            },
        )
        self.assertEqual(
            self.ready_endpoint["broker_features"],
            [
                "transport.v1",
                "p0.control",
                "p1.observability_artifacts",
                "p1.filtered_network_capture",
                "p1.network_batch_transport",
            ],
        )
        self.assertEqual(self.ready_endpoint["protocol_version"], 1)
        self.assertEqual(self.ready_endpoint["schema_version"], 1)
        trusted = await asyncio.to_thread(
            self._http_discovery, self.extension_origin, "POST"
        )
        self.assertEqual(trusted["status"], 200)
        self.trusted_discovery_status = trusted["status"]
        self.trusted_discovery = trusted["payload"]
        self.control_url = self.trusted_discovery["ws_url"]
        self.old_token = self._query_token(self.control_url)
        self.assertTrue(self.old_token)

    async def _stop_broker(self) -> None:
        if self.broker is None:
            return
        broker = self.broker
        self.broker = None
        if broker.poll() is None:
            broker.terminate()
            try:
                await asyncio.to_thread(broker.wait, 10)
            except subprocess.TimeoutExpired:
                broker.kill()
                await asyncio.to_thread(broker.wait, 5)
        if self.broker_stderr_thread is not None:
            self.broker_stderr_thread.join(timeout=2)
        if self.broker_stdout_thread is not None:
            self.broker_stdout_thread.join(timeout=2)
        if broker.stdout is not None:
            broker.stdout.close()
        if broker.stderr is not None:
            broker.stderr.close()
        self._log(
            "broker_stopped",
            {"pid": broker.pid, "returncode": broker.returncode},
        )

    def _drain_broker_stderr(self, stream: object) -> None:
        for line in stream:  # type: ignore[union-attr]
            self.broker_stderr.append(self._redact(str(line).rstrip()))
            del self.broker_stderr[:-80]

    def _drain_broker_stdout(self, stream: object) -> None:
        for line in stream:  # type: ignore[union-attr]
            self.broker_stdout_lines.put(str(line))

    async def _wait_broker_stdout_line(self, timeout: float = 15) -> str:
        deadline = asyncio.get_running_loop().time() + timeout
        while asyncio.get_running_loop().time() < deadline:
            try:
                return self.broker_stdout_lines.get_nowait()
            except queue.Empty:
                if self.broker is not None and self.broker.poll() is not None:
                    break
                await asyncio.sleep(0.05)
        return ""

    async def _launch_context(
        self,
        name: str,
        *,
        keep: bool = True,
        wait_for_worker: bool = True,
    ) -> BrowserContext:
        profile_dir = self.temp_root / name
        extension_arg = str(self.extension_copy.resolve())
        context = await asyncio.wait_for(
            self.playwright.chromium.launch_persistent_context(
                profile_dir,
                executable_path=self.chromium_path,
                headless=False,
                ignore_default_args=["--disable-extensions"],
                args=[
                    f"--disable-extensions-except={extension_arg}",
                    f"--load-extension={extension_arg}",
                    "--window-position=-32000,-32000",
                    "--start-minimized",
                ],
            ),
            timeout=20,
        )
        if keep:
            try:
                worker = (
                    await self._wait_worker(context)
                    if wait_for_worker
                    else None
                )
            except Exception:
                await asyncio.wait_for(context.close(), timeout=10)
                raise
            self.contexts[name] = context
            if worker is not None:
                self.workers[name] = worker
        return context

    async def _find_live_worker(self, context: BrowserContext) -> Worker | None:
        """Probe existing worker handles before relying on a new event."""
        for worker in reversed(context.service_workers):
            try:
                await self._evaluate_worker(worker, "true", timeout=3)
            except Exception:
                continue
            return worker
        return None

    async def _wait_worker(self, context: BrowserContext) -> Worker:
        """Reconcile Playwright's worker list with service-worker events.

        Persistent Chromium profiles can expose a live cached worker without
        emitting a new ``serviceworker`` event. A live evaluation is the
        authority; the event is only a discovery hint.
        """
        deadline = time.monotonic() + 15
        event_task = asyncio.create_task(context.wait_for_event("serviceworker"))
        event_received = False
        try:
            while time.monotonic() < deadline:
                worker = await self._find_live_worker(context)
                if worker is not None:
                    self._log(
                        "service_worker_selected",
                        {
                            "url": worker.url,
                            "source": "existing_live"
                            if not event_received
                            else "event_or_existing_live",
                        },
                    )
                    return worker

                if event_task.done():
                    event_received = True
                    with contextlib.suppress(BaseException):
                        candidate = event_task.result()
                        await self._evaluate_worker(candidate, "true", timeout=3)
                        self._log(
                            "service_worker_selected",
                            {"url": candidate.url, "source": "serviceworker_event"},
                        )
                        return candidate
                    event_task = asyncio.create_task(
                        context.wait_for_event("serviceworker")
                    )
                await asyncio.sleep(0.25)
        finally:
            if not event_task.done():
                event_task.cancel()
            with contextlib.suppress(BaseException):
                await event_task

        broker_sessions = []
        if hasattr(self, "control_url"):
            with contextlib.suppress(Exception):
                broker_sessions = (await self._list_sessions()).get("sessions", [])
        self._log(
            "service_worker_unavailable",
            {
                "error": "no live worker after bounded event/list reconciliation",
                "event_received": event_received,
                "worker_urls": [worker.url for worker in context.service_workers],
                "page_urls": [page.url for page in context.pages],
                "broker_sessions": self._safe_sessions(broker_sessions),
            },
        )
        raise AssertionError(
            "Chrome extension service worker was not live after bounded event/list reconciliation"
        )

    async def _refresh_worker(self, name: str) -> Worker:
        """Use the current worker after a persistent Profile restart."""
        context = self.contexts[name]
        worker = await self._find_live_worker(context)
        if worker is not None:
            self.workers[name] = worker
            self._log(
                "service_worker_selected",
                {"profile": name, "url": worker.url, "source": "refresh_existing_live"},
            )
            return worker
        try:
            worker = await self._wait_worker(context)
        except Exception as error:  # noqa: BLE001
            self._log(
                "service_worker_unavailable",
                {
                    "profile": name,
                    "error": str(error),
                    "worker_urls": [worker.url for worker in context.service_workers],
                    "page_urls": [page.url for page in context.pages],
                },
            )
            raise AssertionError(
                f"Profile {name} did not expose a live extension service worker: {error}"
            ) from error
        self.workers[name] = worker
        self._log("service_worker_selected", {"profile": name, "url": worker.url})
        return worker

    async def _wait_stream(self, name: str) -> int:
        worker = self.workers[name]
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            try:
                connected = await self._evaluate_worker(
                    worker,
                    "({open: typeof streamWs !== 'undefined' && "
                    "streamWs !== null && streamWs.readyState === WebSocket.OPEN, "
                    "generation: typeof lastStreamGeneration !== 'undefined' "
                    "? lastStreamGeneration : null})",
                    timeout=2,
                )
            except asyncio.TimeoutError:
                connected = {}
            except Exception:
                connected = {}
            if (
                isinstance(connected, dict)
                and connected.get("open")
                and isinstance(connected.get("generation"), int)
            ):
                generation = connected["generation"]
                self._log(
                    "stream_hello_transport_ready",
                    {"profile": name, "generation": generation},
                )
                return generation
            await asyncio.sleep(0.25)
        try:
            diagnostics = await self._evaluate_worker(
                worker,
                "({bridge: lastBridgeStatus, projectNeutral: cachedBrokerProjectNeutral, "
                "projectRoot: cachedProjectRoot, frameWs: Boolean(extensionFrameWsUrl), "
                "streamState: streamWs ? streamWs.readyState : null, "
                "streamGeneration: typeof lastStreamGeneration !== 'undefined' "
                "? lastStreamGeneration : null, screencastActive, streamSessionTabId})",
                timeout=2,
            )
        except Exception as error:  # noqa: BLE001
            diagnostics = {"worker_error": str(error)}
        self._log(
            "stream_wait_failed",
            {"profile": name, "worker": diagnostics, "broker_stderr": self.broker_stderr[-20:]},
        )
        raise AssertionError(f"Profile {name} did not keep its Rust stream WebSocket open")

    async def _force_stream(self, name: str) -> None:
        worker = self.workers[name]
        script = """async () => {
            const step = async (name, callback, timeout = 7000) => {
                let timer;
                try {
                    const result = await Promise.race([
                        Promise.resolve().then(callback).then(value => ({ok: true, value})),
                        new Promise(resolve => {
                            timer = setTimeout(() => resolve({ok: false, timeout: name}), timeout);
                        }),
                    ]);
                    return {name, ...result};
                } catch (error) {
                    return {name, ok: false, error: String(error)};
                } finally {
                    if (timer) clearTimeout(timer);
                }
            };
            const probe = await step("discovery_probe", async () => {
                const controller = new AbortController();
                const abortTimer = setTimeout(() => controller.abort(), 5000);
                try {
                    const response = await fetch(DISCOVERY_URL, {
                        method: "POST",
                        headers: { "Content-Type": "application/json" },
                        body: "{}",
                        cache: "no-store",
                        signal: controller.signal,
                    });
                    const body = await response.text();
                    const info = body ? JSON.parse(body) : {};
                    return {
                        status: response.status,
                        ok: response.ok,
                        type: response.type,
                        bodyLength: body.length,
                        keys: Object.keys(info).sort(),
                        brokerFeatures: info.broker_features || [],
                        hasTokenizedWs: String(info.ws_url || "").includes("token="),
                        hasTokenizedFrameWs: String(info.extension_frame_ws_url || "").includes("token="),
                    };
                } finally {
                    clearTimeout(abortTimer);
                }
            });
            const discovery = await step("refresh_bridge_cache", refreshBridgeCache);
            const heartbeat = await step(
                "heartbeat_once",
                () => heartbeatOnce({forceStream: true}),
                10000,
            );
            return {
                probe,
                discovery,
                heartbeat,
                bridge: lastBridgeStatus,
                projectNeutral: cachedBrokerProjectNeutral,
                frameWs: Boolean(extensionFrameWsUrl),
                streamState: streamWs ? streamWs.readyState : null,
                streamGeneration: typeof lastStreamGeneration !== "undefined"
                    ? lastStreamGeneration : null,
                screencastActive,
            };
        }"""
        try:
            result = await self._evaluate_worker(worker, script, timeout=20)
        except asyncio.TimeoutError as error:
            try:
                diagnostics = await self._evaluate_worker(
                    worker,
                    "({bridge: lastBridgeStatus, projectNeutral: cachedBrokerProjectNeutral, "
                    "projectRoot: cachedProjectRoot, frameWs: Boolean(extensionFrameWsUrl), "
                    "streamState: streamWs ? streamWs.readyState : null, "
                    "streamGeneration: typeof lastStreamGeneration !== 'undefined' "
                    "? lastStreamGeneration : null, screencastActive})",
                    timeout=2,
                )
            except Exception as diagnostic_error:  # noqa: BLE001
                diagnostics = {"worker_error": str(diagnostic_error)}
            self._log(
                "extension_force_stream_timeout",
                {"profile": name, "worker": diagnostics, "broker_stderr": self.broker_stderr[-20:]},
            )
            raise AssertionError(
                f"Profile {name} did not refresh its Rust stream within 20 seconds"
            ) from error
        self._log("extension_force_stream", {"profile": name, "result": result})

    async def _control(
        self,
        payload: dict,
        *,
        url: str | None = None,
        timeout: float = 5,
    ) -> dict:
        control_url = url or self.control_url
        async with websockets.connect(
            control_url,
            origin=self.extension_origin,
            open_timeout=5,
            close_timeout=2,
            max_size=2 * 1024 * 1024,
        ) as socket:
            await socket.send(json.dumps(payload))
            message = await asyncio.wait_for(socket.recv(), timeout=timeout)
        if isinstance(message, bytes):
            message = message.decode("utf-8")
        return json.loads(message)

    async def _acquire_lease(self, target: dict, owner: str) -> str:
        request_id = f"rust-lease-{owner}-{uuid.uuid4().hex}"
        response = await self._control(
            {
                "schema_version": 1,
                "request_id": request_id,
                "caller_label": owner,
                "project_root": str(self.temp_root),
                "cmd": "acquire_browser_lease",
                "extension_instance_id": target["extension_instance_id"],
                "owner_label": owner,
                "ttl_secs": 60,
            }
        )
        self.assertTrue(response.get("ok"), response)
        self.assertEqual(response.get("request_id"), request_id)
        self.assertEqual(response.get("operation"), "acquire_browser_lease")
        token = response.get("lease_token")
        self.assertIsInstance(token, str)
        self._log(
            "lease_acquired",
            {
                "profile": target["extension_instance_id"],
                "owner": owner,
                "request_id": request_id,
            },
        )
        return token

    async def _release_lease(self, target: dict, owner: str, token: str) -> None:
        request_id = f"rust-release-{owner}-{uuid.uuid4().hex}"
        response = await self._control(
            {
                "schema_version": 1,
                "request_id": request_id,
                "caller_label": owner,
                "project_root": str(self.temp_root),
                "cmd": "release_browser_lease",
                "extension_instance_id": target["extension_instance_id"],
                "lease_token": token,
            }
        )
        self.assertTrue(response.get("ok"), response)
        self.assertEqual(response.get("request_id"), request_id)
        self.assertEqual(response.get("operation"), "release_browser_lease")
        self._log(
            "lease_released",
            {
                "profile": target["extension_instance_id"],
                "owner": owner,
                "request_id": request_id,
            },
        )

    async def _wait_target_url(self, target: dict, expected_url: str) -> None:
        deadline = time.monotonic() + 15
        last_sessions: list[dict] = []
        while time.monotonic() < deadline:
            response = await self._list_sessions()
            last_sessions = response.get("sessions", [])
            for session in last_sessions:
                if (
                    session.get("identity", {}).get("extension_instance_id")
                    != target["extension_instance_id"]
                ):
                    continue
                for window in session.get("windows", []):
                    for tab in window.get("tabs", []):
                        if (
                            str(tab.get("window_id")) == str(target["window_id"])
                            and str(tab.get("id")) == str(target["tab_id"])
                            and tab.get("url") == expected_url
                        ):
                            return
            await asyncio.sleep(0.2)
        raise AssertionError(
            f"Rust Broker did not publish {expected_url!r} for target {target}: "
            f"{self._redact(json.dumps(self._safe_sessions(last_sessions), sort_keys=True))}"
        )

    async def _list_sessions(self) -> dict:
        return await self._control(
            {
                "schema_version": 1,
                "request_id": f"list-{uuid.uuid4().hex}",
                "caller_label": "rust-transport-test",
                "project_root": str(self.temp_root),
                "cmd": "list_browser_sessions",
            }
        )

    async def _wait_sessions(
        self, expected_ready: int = 2, *, expected_count: int | None = None
    ) -> list[dict]:
        deadline = time.monotonic() + 30
        last: dict = {}
        while time.monotonic() < deadline:
            last = await self._list_sessions()
            sessions = last.get("sessions", [])
            ready = [item for item in sessions if item.get("health") == "ready"]
            if len(ready) >= expected_ready and (
                expected_count is None or len(ready) == expected_count
            ):
                return ready
            await asyncio.sleep(0.5)
        raise AssertionError(
            f"Rust Broker did not reach {expected_ready} ready sessions: "
            f"{self._redact(json.dumps(last, sort_keys=True))}; "
            f"stderr={self.broker_stderr[-10:]}"
        )

    async def _wait_session_stream_generation(
        self, instance_id: str, *, greater_than: int | None = None
    ) -> dict:
        deadline = time.monotonic() + 30
        last: dict = {}
        while time.monotonic() < deadline:
            last = await self._list_sessions()
            for session in last.get("sessions", []):
                if session.get("identity", {}).get("extension_instance_id") != instance_id:
                    continue
                generation = session.get("stream_generation")
                if (
                    session.get("health") == "ready"
                    and isinstance(generation, int)
                    and (greater_than is None or generation > greater_than)
                ):
                    return session
            await asyncio.sleep(0.5)
        raise AssertionError(
            f"Rust Broker did not expose a new stream generation for {instance_id}: "
            f"{self._redact(json.dumps(last, sort_keys=True))}; "
            f"stderr={self.broker_stderr[-10:]}"
        )

    @staticmethod
    def _session_for_path(sessions: list[dict], path: str) -> dict:
        for session in sessions:
            tabs = [
                tab
                for window in session.get("windows", [])
                for tab in window.get("tabs", [])
            ]
            if any(path in str(tab.get("url", "")) for tab in tabs):
                return session
        raise AssertionError(f"no session contains page path {path}: {sessions!r}")

    async def test_locator_contexts_are_reverified_in_real_chrome(self) -> None:
        sessions = await self._wait_sessions(2)
        target = self._target(self._session_for_path(sessions, "/rust-a"))
        owner = "rust-locator-context"
        token = await self._acquire_lease(target, owner)
        try:
            url = f"http://127.0.0.1:{self.http.server_address[1]}/locator-context"
            navigation = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-locator-context-navigation",
                    "caller_label": owner,
                    "project_root": str(self.temp_root),
                    "cmd": "navigate",
                    "target": target,
                    "lease_token": token,
                    "url": url,
                    "timeout_ms": 15_000,
                }
            )
            self.assertTrue(navigation.get("ok"), navigation)
            await self._wait_target_url(target, url)

            snapshot = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-locator-context-snapshot",
                    "caller_label": owner,
                    "project_root": str(self.temp_root),
                    "cmd": "get_page_snapshot",
                    "target": target,
                    "lease_token": token,
                }
            )
            self.assertTrue(snapshot.get("ok"), snapshot)
            revision = snapshot["page_context_revision"]
            snapshot_id = snapshot["snapshot_id"]
            elements = snapshot["interactive_elements"]
            page = self.contexts["profile-a"].pages[0]
            shadow_probe = await page.evaluate(
                """() => ({
                    host: Boolean(document.querySelector('#shadow-widget')),
                    shadow: Boolean(document.querySelector('#shadow-widget')?.shadowRoot),
                    button: Boolean(document.querySelector('#shadow-widget')?.shadowRoot?.querySelector('#shadow-submit')),
                })"""
            )
            self._log(
                "locator_context_probe",
                {"shadow_probe": shadow_probe, "element_ids": [item.get("id") for item in elements]},
            )
            frame_element = next(
                item for item in elements if item.get("id") == "frame-submit"
            )
            css_element = next(
                item
                for item in elements
                if item.get("attributes", {}).get("class") == "css-only-target"
            )
            shadow_element = next(
                item for item in elements if item.get("id") == "shadow-submit"
            )
            email_element = next(
                item for item in elements if item.get("id") == "context-email"
            )
            self.assertEqual(email_element.get("label"), "Email address")
            self.assertEqual(email_element.get("placeholder"), "name@example.test")
            self.assertEqual(
                email_element.get("attributes", {}).get("data-qa"), "context-email"
            )
            self.assertIsNotNone(frame_element.get("context", {}).get("frame"))
            self.assertIsNone(frame_element.get("context", {}).get("shadow_root"))
            self.assertEqual(
                shadow_element.get("context", {}).get("shadow_root"),
                "#shadow-widget",
            )

            actions = []
            for request_id, element in [
                ("rust-locator-context-frame-click", frame_element),
                ("rust-locator-context-css-click", css_element),
                ("rust-locator-context-shadow-click", shadow_element),
            ]:
                action = await self._control(
                    {
                        "schema_version": 1,
                        "request_id": request_id,
                        "caller_label": owner,
                        "project_root": str(self.temp_root),
                        "cmd": "execute_browser_action",
                        "target": target,
                        "lease_token": token,
                        "action": "click",
                        "element": {
                            "reference": element["ref"],
                            "snapshot_id": snapshot_id,
                            "page_context_revision": revision,
                        },
                    }
                )
                self.assertTrue(action.get("ok"), action)
                expected_kind = "css" if element is css_element else "role"
                self.assertEqual(
                    action["action_outcome"]["candidate"]["kind"], expected_kind
                )
                actions.append(action)

            frame = page.frame(name="checkout-frame")
            self.assertIsNotNone(frame)
            self.assertEqual(await frame.locator("#frame-click-count").text_content(), "1")
            self.assertEqual(await page.locator("#css-click-count").text_content(), "1")
            self.assertEqual(
                await page.evaluate(
                    "document.querySelector('#shadow-widget').shadowRoot.querySelector('#shadow-click-count').textContent"
                ),
                "1",
            )
            changed_url = f"{url}?revision-change=1"
            changed_navigation = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-locator-context-revision-change",
                    "caller_label": owner,
                    "project_root": str(self.temp_root),
                    "cmd": "navigate",
                    "target": target,
                    "lease_token": token,
                    "url": changed_url,
                    "timeout_ms": 15_000,
                }
            )
            self.assertTrue(changed_navigation.get("ok"), changed_navigation)
            await self._wait_target_url(target, changed_url)
            stale_after_navigation = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-locator-context-stale-after-navigation",
                    "caller_label": owner,
                    "project_root": str(self.temp_root),
                    "cmd": "execute_browser_action",
                    "target": target,
                    "lease_token": token,
                    "action": "click",
                    "element": {
                        "reference": frame_element["ref"],
                        "snapshot_id": snapshot_id,
                        "page_context_revision": revision,
                    },
                }
            )
            self.assertFalse(stale_after_navigation.get("ok"), stale_after_navigation)
            self.assertIn(
                stale_after_navigation.get("code"),
                {"stale_element_reference", "stale_browser_target"},
                stale_after_navigation,
            )
            self._log(
                "locator_contexts_verified",
                {
                    "frame": frame_element["context"].get("frame"),
                    "shadow_root": shadow_element["context"].get("shadow_root"),
                    "revision_change_error": stale_after_navigation.get("code"),
                    "actions": [
                        {
                            "request_id": item["request_id"],
                            "candidate_kind": item["action_outcome"]["candidate"]["kind"],
                        }
                        for item in actions
                    ],
                    "playwright_mutations": 0,
                },
            )
        finally:
            await self._release_lease(target, owner, token)

    async def test_stage_five_real_evidence_and_bounded_backpressure(self) -> None:
        self._set_stage("stage 5.6 real screenshot, Console, and Network evidence")
        sessions = await self._wait_sessions(2, expected_count=2)
        session_a = self._session_for_path(sessions, "/rust-a")
        session_b = self._session_for_path(sessions, "/rust-b")
        target_a = self._target(session_a)
        target_b = self._target(session_b)
        owner_a = "rust-stage5-6-profile-a"
        owner_b = "rust-stage5-6-profile-b"
        tokens = await asyncio.gather(
            self._acquire_lease(target_a, owner_a),
            self._acquire_lease(target_b, owner_b),
        )
        leases_valid = True
        network_active = False
        metrics: dict[str, object] = {
            "operations_ms": {},
            "working_set_bytes": [],
        }

        def sample_working_set(label: str) -> None:
            samples = metrics["working_set_bytes"]
            assert isinstance(samples, list)
            samples.append(
                {
                    "label": label,
                    "bytes": self._working_set_bytes(
                        self.broker.pid if self.broker is not None else None
                    ),
                }
            )

        async def control(label: str, payload: dict, *, timeout: float = 20) -> dict:
            started = time.perf_counter()
            result = await self._control(payload, timeout=timeout)
            elapsed = round((time.perf_counter() - started) * 1000, 1)
            operations = metrics["operations_ms"]
            assert isinstance(operations, dict)
            operations[label] = elapsed
            sample_working_set(label)
            return result

        try:
            sample_working_set("before-evidence")
            screenshot_a, screenshot_b = await asyncio.gather(
                control(
                    "screenshot_png_profile_a",
                    {
                        "schema_version": 1,
                        "request_id": "rust-stage5-6-screenshot-a",
                        "caller_label": owner_a,
                        "project_root": str(self.temp_root),
                        "cmd": "capture_browser_screenshot",
                        "target": target_a,
                        "lease_token": tokens[0],
                        "format": "png",
                        "full_page": False,
                    },
                ),
                control(
                    "screenshot_jpeg_profile_b",
                    {
                        "schema_version": 1,
                        "request_id": "rust-stage5-6-screenshot-b",
                        "caller_label": owner_b,
                        "project_root": str(self.temp_root),
                        "cmd": "capture_browser_screenshot",
                        "target": target_b,
                        "lease_token": tokens[1],
                        "format": "jpeg",
                        "quality": 75,
                        "full_page": True,
                    },
                ),
            )
            self.assertTrue(screenshot_a.get("ok"), screenshot_a)
            self.assertTrue(screenshot_b.get("ok"), screenshot_b)
            artifact_root = self.temp_root / ".teshi" / "artifacts" / "browser"
            screenshot_paths = [
                artifact_root / screenshot_a["artifact"]["path"],
                artifact_root / screenshot_b["artifact"]["path"],
            ]
            self.assertEqual(screenshot_paths[0].read_bytes()[:8], b"\x89PNG\r\n\x1a\n")
            self.assertEqual(screenshot_paths[1].read_bytes()[:2], b"\xff\xd8")
            self.assertTrue(all(path.stat().st_size > 0 for path in screenshot_paths))
            if self.artifact_dir is not None:
                shutil.copyfile(
                    screenshot_paths[0], self.artifact_dir / "rust-stage5-6-profile-a.png"
                )
                shutil.copyfile(
                    screenshot_paths[1], self.artifact_dir / "rust-stage5-6-profile-b.jpg"
                )

            console_start = await control(
                "console_start_profile_a",
                {
                    "schema_version": 1,
                    "request_id": "rust-stage5-6-console-start",
                    "caller_label": owner_a,
                    "project_root": str(self.temp_root),
                    "cmd": "start_console_capture",
                    "target": target_a,
                    "lease_token": tokens[0],
                    "levels": ["info", "error"],
                    "max_entries": 8,
                    "max_bytes": 4096,
                    "max_age_ms": 60_000,
                    "sensitive_fields": ["token"],
                },
            )
            self.assertTrue(console_start.get("ok"), console_start)
            await self.contexts["profile-a"].pages[0].evaluate(
                """() => {
                    console.info('stage5 token=rust-console-secret');
                    console.error('stage5-large-' + 'x'.repeat(70000));
                }"""
            )
            console_list: dict = {}
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                console_list = await control(
                    "console_list_profile_a",
                    {
                        "schema_version": 1,
                        "request_id": f"rust-stage5-6-console-list-{uuid.uuid4().hex}",
                        "caller_label": owner_a,
                        "project_root": str(self.temp_root),
                        "cmd": "list_console_events",
                        "target": target_a,
                        "lease_token": tokens[0],
                    },
                )
                if console_list.get("events"):
                    break
                await asyncio.sleep(0.25)
            self.assertTrue(console_list.get("events"), console_list)
            console_text = json.dumps(console_list, ensure_ascii=False)
            self.assertNotIn("rust-console-secret", console_text)
            self.assertLessEqual(console_list.get("retained_bytes", 0), 4096)
            self.assertLessEqual(console_list.get("retained_entries", 0), 8)
            diagnostics = console_list.get("diagnostics", {})
            self.assertGreaterEqual(diagnostics.get("truncated_events", 0), 1)
            console_stop = await control(
                "console_stop_profile_a",
                {
                    "schema_version": 1,
                    "request_id": "rust-stage5-6-console-stop",
                    "caller_label": owner_a,
                    "project_root": str(self.temp_root),
                    "cmd": "stop_console_capture",
                    "target": target_a,
                    "lease_token": tokens[0],
                },
            )
            self.assertTrue(console_stop.get("ok"), console_stop)

            network_start = await control(
                "network_start_profile_b",
                {
                    "schema_version": 1,
                    "request_id": "rust-stage5-6-network-start",
                    "caller_label": owner_b,
                    "project_root": str(self.temp_root),
                    "cmd": "start_network_capture",
                    "target": target_b,
                    "lease_token": tokens[1],
                    "allowed_hostnames": ["127.0.0.1"],
                    "capture_request_bodies": True,
                    "max_request_body_bytes": 64,
                    "max_body_bytes": 1024,
                    "max_entries": 256,
                    "max_bytes": 256 * 1024,
                    "sensitive_fields": ["token"],
                },
            )
            self.assertTrue(network_start.get("ok"), network_start)
            network_active = True
            network_port = self.http.server_address[1]
            await self.contexts["profile-b"].pages[0].evaluate(
                """async ({port}) => {
                    const response = await fetch(
                        `http://127.0.0.1:${port}/network-match?token=rust-network-secret`,
                        {
                            method: 'POST',
                            headers: {'Content-Type': 'text/plain'},
                            body: 'request-body-' + 'q'.repeat(4096),
                        },
                    );
                    await response.text();
                    await fetch(`http://localhost:${port}/network-filtered`).catch(() => null);
                }""",
                {"port": network_port},
            )
            network_list: dict = {}
            matching_request: dict | None = None
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                network_list = await control(
                    "network_list_profile_b",
                    {
                        "schema_version": 1,
                        "request_id": f"rust-stage5-6-network-list-{uuid.uuid4().hex}",
                        "caller_label": owner_b,
                        "project_root": str(self.temp_root),
                        "cmd": "list_network_requests",
                        "target": target_b,
                        "lease_token": tokens[1],
                    },
                )
                requests = network_list.get("requests", [])
                matching_request = next(
                    (
                        item
                        for item in requests
                        if "/network-match" in str(item.get("url", ""))
                    ),
                    None,
                )
                if matching_request is not None:
                    break
                await asyncio.sleep(0.25)
            self.assertIsNotNone(matching_request, network_list)
            assert matching_request is not None
            self.assertNotIn("request_body", matching_request)
            self.assertNotIn("rust-network-secret", json.dumps(network_list))
            self.assertTrue(
                all("network-filtered" not in str(item.get("url", "")) for item in network_list["requests"])
            )
            detail = await control(
                "network_detail_profile_b",
                {
                    "schema_version": 1,
                    "request_id": "rust-stage5-6-network-detail",
                    "caller_label": owner_b,
                    "project_root": str(self.temp_root),
                    "cmd": "get_network_request_detail",
                    "target": target_b,
                    "lease_token": tokens[1],
                    "network_request_id": matching_request["request_id"],
                    "include_body": True,
                    "max_body_bytes": 1024,
                },
            )
            self.assertTrue(detail.get("ok"), detail)
            request_body = detail["request"].get("request_body", {})
            self.assertLessEqual(request_body.get("captured_size", 0), 64)
            self.assertLessEqual(detail.get("returned_size", 0), 1024)
            self.assertTrue(detail.get("truncated"))
            self.assertNotIn("rust-network-secret", json.dumps(detail))

            self._set_stage("stage 5.6 broker-offline Network burst and queue bounds")
            old_port = self.discovery_port
            await self._stop_broker()
            leases_valid = False
            await self._wait_port_closed(old_port)
            burst_count = 500
            await self.contexts["profile-b"].pages[0].evaluate(
                """async ({port, count}) => {
                    await Promise.all(
                        Array.from({length: count}, (_, index) =>
                            fetch(`http://127.0.0.1:${port}/network-burst-${index}`, {
                                method: 'POST',
                                headers: {'Content-Type': 'text/plain'},
                                body: 'burst-' + 'b'.repeat(256),
                            }).then((response) => response.text()).catch(() => ''),
                        ),
                    );
                }""",
                {"port": network_port, "count": burst_count},
            )
            queue_state: list[dict] = []
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                try:
                    observed = await self._evaluate_worker(
                        self.workers["profile-b"],
                        "Array.from(networkDeliveryStates.values()).map(state => ({queue_length: state.queue.length, queue_bytes: state.queue_bytes, dropped_events: state.dropped_events_total, dropped_bytes: state.dropped_bytes_total, active: state.active}))",
                        timeout=3,
                    )
                    queue_state = observed if isinstance(observed, list) else []
                except Exception:
                    queue_state = []
                if queue_state and (
                    queue_state[0].get("dropped_events", 0) > 0
                    or queue_state[0].get("queue_length", 0) >= 1_000
                ):
                    break
                await asyncio.sleep(0.25)
            self.assertTrue(queue_state, queue_state)
            queue = queue_state[0]
            self.assertLessEqual(queue["queue_length"], 1_000)
            self.assertLessEqual(queue["queue_bytes"], 5 * 1024 * 1024)
            self.assertGreater(queue["dropped_events"], 0, queue_state)
            metrics["backpressure"] = {
                "burst_requests": burst_count,
                "queue": queue,
            }
            sample_working_set("broker-offline-after-burst")

            self.discovery_port = old_port
            await self._start_broker()
            await asyncio.gather(
                self._force_stream("profile-a"),
                self._force_stream("profile-b"),
            )
            recovered = await self._wait_sessions(2, expected_count=2)
            recovered_a = self._session_for_path(recovered, "/rust-a")
            recovered_b = self._session_for_path(recovered, "/rust-b")
            self.assertIsInstance(recovered_a["stream_generation"], int)
            self.assertIsInstance(recovered_b["stream_generation"], int)
            self.assertGreaterEqual(recovered_a["stream_generation"], 1)
            self.assertGreaterEqual(recovered_b["stream_generation"], 1)
            recovered_worker_state: dict = {}
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                try:
                    observed = await self._evaluate_worker(
                        self.workers["profile-b"],
                        "({captures: networkCapturesByTab.size, deliveries: networkDeliveryStates.size, generation: lastStreamGeneration})",
                        timeout=3,
                    )
                    recovered_worker_state = observed if isinstance(observed, dict) else {}
                except Exception:
                    recovered_worker_state = {}
                if (
                    recovered_worker_state.get("captures") == 0
                    and recovered_worker_state.get("deliveries") == 0
                ):
                    break
                await asyncio.sleep(0.25)
            self.assertEqual(recovered_worker_state.get("captures"), 0, recovered_worker_state)
            self.assertEqual(recovered_worker_state.get("deliveries"), 0, recovered_worker_state)
            self.assertGreaterEqual(recovered_worker_state.get("generation", 0), 1)
            metrics["reconnect_generations"] = {
                "profile-a": recovered_a["stream_generation"],
                "profile-b": recovered_b["stream_generation"],
                "extension_cleanup": recovered_worker_state,
            }
            self._log("stage5_6_real_evidence", metrics)
        finally:
            if network_active and leases_valid and self.broker is not None:
                with contextlib.suppress(Exception):
                    await self._control(
                        {
                            "schema_version": 1,
                            "request_id": "rust-stage5-6-network-stop-cleanup",
                            "caller_label": owner_b,
                            "project_root": str(self.temp_root),
                            "cmd": "stop_network_capture",
                            "target": target_b,
                            "lease_token": tokens[1],
                        }
                    )
            if leases_valid:
                await asyncio.gather(
                    self._release_lease(target_a, owner_a, tokens[0]),
                    self._release_lease(target_b, owner_b, tokens[1]),
                )

    async def test_two_profiles_stay_isolated_across_rust_transport_restart(self) -> None:
        self._set_stage("verify discovery origin and feature boundary")
        public = await asyncio.to_thread(self._http_discovery, None)
        self.assertEqual(public["status"], 200)
        self.assertNotIn("token", json.dumps(public["payload"]))
        trusted_get = await asyncio.to_thread(
            self._http_discovery, self.extension_origin
        )
        self.assertEqual(trusted_get["status"], 200)
        self.assertNotIn("token", json.dumps(trusted_get["payload"]))
        trusted = self.trusted_discovery
        self.assertIn("token=", trusted["ws_url"])
        self.assertEqual(
            trusted["broker_features"],
            [
                "transport.v1",
                "p0.control",
                "p1.observability_artifacts",
                "p1.filtered_network_capture",
                "p1.network_batch_transport",
            ],
        )
        untrusted = await asyncio.to_thread(
            self._http_discovery,
            "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbc",
        )
        self.assertEqual(untrusted["status"], 403)
        self._log(
            "discovery_verified",
            {
                "public_status": public["status"],
                "trusted_get_status": trusted_get["status"],
                "trusted_status": self.trusted_discovery_status,
                "untrusted_status": untrusted["status"],
                "broker_features": trusted["broker_features"],
            },
        )

        self._set_stage("wait heartbeat stream hello and session target registration")
        initial_a_generation, initial_b_generation = await asyncio.gather(
            self._wait_stream("profile-a"), self._wait_stream("profile-b")
        )
        self.initial_stream_generations = {
            "profile-a": initial_a_generation,
            "profile-b": initial_b_generation,
        }
        sessions = await self._wait_sessions(2)
        session_a = self._session_for_path(sessions, "/rust-a")
        session_b = self._session_for_path(sessions, "/rust-b")
        self.assertNotEqual(
            session_a["identity"]["extension_instance_id"],
            session_b["identity"]["extension_instance_id"],
        )
        self.initial_ids = {
            "profile-a": session_a["identity"]["extension_instance_id"],
            "profile-b": session_b["identity"]["extension_instance_id"],
        }
        self._log(
            "sessions_ready",
            {
                "sessions": self._safe_sessions([session_a, session_b]),
                "extension_instances_distinct": True,
            },
        )

        self._set_stage("verify target ambiguity and lease pre-dispatch gate")
        ambiguous = await self._control(
            {
                "schema_version": 1,
                "request_id": "rust-ambiguous-target",
                "caller_label": "rust-transport-test",
                "project_root": str(self.temp_root),
                "cmd": "get_page_snapshot",
            }
        )
        self.assertEqual(ambiguous["code"], "ambiguous_browser_target")
        target_a = self._target(session_a)
        target_b = self._target(session_b)
        missing_lease = await self._control(
            {
                "schema_version": 1,
                "request_id": "rust-missing-lease",
                "caller_label": "rust-transport-test",
                "project_root": str(self.temp_root),
                "cmd": "get_page_snapshot",
                "target": target_a,
            }
        )
        self.assertEqual(missing_lease["code"], "invalid_browser_lease")
        self._log(
            "control_boundary_verified",
            {
                "implicit_target_error": ambiguous["code"],
                "missing_lease_error": missing_lease["code"],
                "navigation_executed": False,
            },
        )

        self._set_stage("navigate and snapshot both Profiles through Rust P0")
        lease_owners = ["rust-p0-profile-a", "rust-p0-profile-b"]
        tokens = await asyncio.gather(
            self._acquire_lease(target_a, lease_owners[0]),
            self._acquire_lease(target_b, lease_owners[1]),
        )
        try:
            port = self.http.server_address[1]
            navigation_requests = [
                ("rust-navigate-profile-a", target_a, f"http://127.0.0.1:{port}/rust-a-nav"),
                ("rust-navigate-profile-b", target_b, f"http://127.0.0.1:{port}/rust-b-nav"),
            ]
            navigation = await asyncio.gather(
                *(
                    self._control(
                        {
                            "schema_version": 1,
                            "request_id": request_id,
                            "caller_label": lease_owners[index],
                            "project_root": str(self.temp_root),
                            "cmd": "navigate",
                            "target": target,
                            "lease_token": tokens[index],
                            "url": url,
                            "timeout_ms": 15_000,
                        }
                    )
                    for index, (request_id, target, url) in enumerate(navigation_requests)
                )
            )
            for index, (request_id, target, url) in enumerate(navigation_requests):
                result = navigation[index]
                self.assertTrue(result.get("ok"), result)
                self.assertEqual(result.get("request_id"), request_id)
                self.assertEqual(result.get("operation"), "navigate")
                self.assertEqual(result.get("extension_instance_id"), target["extension_instance_id"])
                self.assertEqual(result.get("target"), target)
                self.assertEqual(result.get("action_outcome", {}).get("url"), url)
            await asyncio.gather(
                self._wait_target_url(target_a, navigation_requests[0][2]),
                self._wait_target_url(target_b, navigation_requests[1][2]),
            )
            self._log(
                "navigation_verified",
                {
                    "request_ids": [item[0] for item in navigation_requests],
                    "profiles": [target_a["extension_instance_id"], target_b["extension_instance_id"]],
                    "stream_generations": [
                        session_a["stream_generation"],
                        session_b["stream_generation"],
                    ],
                },
            )

            snapshot_requests = [
                ("rust-snapshot-profile-a", target_a, lease_owners[0], tokens[0], "rust-a-nav"),
                ("rust-snapshot-profile-b", target_b, lease_owners[1], tokens[1], "rust-b-nav"),
            ]
            snapshots = await asyncio.gather(
                *(
                    self._control(
                        {
                            "schema_version": 1,
                            "request_id": request_id,
                            "caller_label": owner,
                            "project_root": str(self.temp_root),
                            "cmd": "get_page_snapshot",
                            "target": target,
                            "lease_token": token,
                        }
                    )
                    for request_id, target, owner, token, _ in snapshot_requests
                )
            )
            snapshot_refs = {}
            for index, (request_id, target, _, _, profile) in enumerate(snapshot_requests):
                snapshot = snapshots[index]
                self.assertTrue(snapshot.get("ok"), snapshot)
                self.assertEqual(snapshot.get("request_id"), request_id)
                self.assertEqual(snapshot.get("operation"), "get_page_snapshot")
                self.assertEqual(snapshot.get("extension_instance_id"), target["extension_instance_id"])
                self.assertEqual(snapshot.get("target"), target)
                self.assertEqual(snapshot.get("url"), navigation_requests[index][2])
                self.assertEqual(snapshot.get("title"), f"Rust {profile}")
                self.assertEqual(snapshot.get("snapshot_id"), request_id)
                self.assertTrue(snapshot.get("page_context_revision"))
                self.assertTrue(
                    any(
                        f"Snapshot {profile}" in str(element.get("text"))
                        for element in snapshot.get("interactive_elements", [])
                    ),
                    snapshot,
                )
                pointer_element = next(
                    (
                        element
                        for element in snapshot.get("interactive_elements", [])
                        if element.get("id") == "pointer-click"
                    ),
                    None,
                )
                self.assertIsNotNone(pointer_element, snapshot)
                self.assertTrue(pointer_element.get("ref", "").startswith("@e"), pointer_element)
                snapshot_refs[target["extension_instance_id"]] = pointer_element
            self.assertNotEqual(
                snapshots[0]["url"], snapshots[1]["url"], "Profile pages crossed routing"
            )
            self.assertNotEqual(
                snapshots[0]["extension_instance_id"],
                snapshots[1]["extension_instance_id"],
                "Snapshot responses crossed Profile identity",
            )
            self._log(
                "snapshot_verified",
                {
                    "request_ids": [item[0] for item in snapshot_requests],
                    "profiles": [item[1]["extension_instance_id"] for item in snapshot_requests],
                    "urls": [snapshot["url"] for snapshot in snapshots],
                    "snapshot_ids": [snapshot["snapshot_id"] for snapshot in snapshots],
                },
            )

            revision_a = snapshots[0]["page_context_revision"]
            revision_b = snapshots[1]["page_context_revision"]
            self._set_stage("verify click pre-dispatch rejection boundaries")
            missing_element = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-click-missing-element",
                    "caller_label": lease_owners[0],
                    "project_root": str(self.temp_root),
                    "cmd": "execute_browser_action",
                    "target": target_a,
                    "lease_token": tokens[0],
                    "action": "click",
                    "element": {
                        "css": "#missing-action",
                        "page_context_revision": revision_a,
                    },
                },
                timeout=10,
            )
            wrong_lease = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-click-wrong-lease",
                    "caller_label": lease_owners[0],
                    "project_root": str(self.temp_root),
                    "cmd": "execute_browser_action",
                    "target": target_b,
                    "lease_token": tokens[0],
                    "action": "click",
                    "element": {
                        "css": "#dom-click",
                        "page_context_revision": revision_b,
                    },
                }
            )
            wrong_profile_target = {
                **target_a,
                "extension_instance_id": "profile-does-not-exist",
            }
            wrong_profile = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-click-wrong-profile",
                    "caller_label": lease_owners[0],
                    "project_root": str(self.temp_root),
                    "cmd": "execute_browser_action",
                    "target": wrong_profile_target,
                    "lease_token": tokens[0],
                    "action": "click",
                    "element": {
                        "css": "#dom-click",
                        "page_context_revision": revision_a,
                    },
                }
            )
            stale_revision = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-click-stale-revision",
                    "caller_label": lease_owners[0],
                    "project_root": str(self.temp_root),
                    "cmd": "execute_browser_action",
                    "target": target_a,
                    "lease_token": tokens[0],
                    "action": "click",
                    "element": {
                        "css": "#dom-click",
                        "page_context_revision": "stale-revision",
                    },
                }
            )
            ambiguous_locator = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-test-id-ambiguous",
                    "caller_label": lease_owners[0],
                    "project_root": str(self.temp_root),
                    "cmd": "execute_browser_action",
                    "target": target_a,
                    "lease_token": tokens[0],
                    "action": "click",
                    "element": {
                        "test_id": "ambiguous-click",
                        "page_context_revision": revision_a,
                    },
                }
            )
            hidden_element = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-hidden-element",
                    "caller_label": lease_owners[0],
                    "project_root": str(self.temp_root),
                    "cmd": "execute_browser_action",
                    "target": target_a,
                    "lease_token": tokens[0],
                    "action": "click",
                    "element": {
                        "css": "#hidden-action",
                        "page_context_revision": revision_a,
                    },
                },
                timeout=10,
            )
            disabled_element = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-disabled-element",
                    "caller_label": lease_owners[0],
                    "project_root": str(self.temp_root),
                    "cmd": "execute_browser_action",
                    "target": target_a,
                    "lease_token": tokens[0],
                    "action": "click",
                    "element": {
                        "css": "#disabled-action",
                        "page_context_revision": revision_a,
                    },
                }
            )
            assertion_failed = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-assertion-failed",
                    "caller_label": lease_owners[0],
                    "project_root": str(self.temp_root),
                    "cmd": "execute_browser_action",
                    "target": target_a,
                    "lease_token": tokens[0],
                    "action": "assert_text",
                    "value": "this text never appears in the fixture",
                    "element": {
                        "css": "#assert-target",
                        "page_context_revision": revision_a,
                    },
                },
                timeout=10,
            )
            timed_out = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-action-wait-timeout",
                    "caller_label": lease_owners[0],
                    "project_root": str(self.temp_root),
                    "cmd": "execute_browser_action",
                    "target": target_a,
                    "lease_token": tokens[0],
                    "action": "click",
                    "wait": {
                        "kind": "visible_text",
                        "text": "this text never appears in the fixture",
                    },
                    "element": {
                        "css": "#timeout-action",
                        "page_context_revision": revision_a,
                    },
                },
                timeout=10,
            )
            cross_profile_reference = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-reference-cross-profile",
                    "caller_label": lease_owners[1],
                    "project_root": str(self.temp_root),
                    "cmd": "execute_browser_action",
                    "target": target_b,
                    "lease_token": tokens[1],
                    "action": "click",
                    "element": {
                        "reference": snapshot_refs[target_a["extension_instance_id"]]["ref"],
                        "snapshot_id": snapshots[0]["snapshot_id"],
                        "page_context_revision": revision_a,
                    },
                }
            )
            self._log(
                "click_rejection_responses",
                {
                    "missing_element": {
                        "ok": missing_element.get("ok"),
                        "code": missing_element.get("code"),
                        "operation": missing_element.get("operation"),
                        "target": missing_element.get("target"),
                        "error": missing_element.get("error"),
                    },
                    "wrong_lease": {
                        "ok": wrong_lease.get("ok"),
                        "code": wrong_lease.get("code"),
                        "operation": wrong_lease.get("operation"),
                        "error": wrong_lease.get("error"),
                    },
                    "wrong_profile": {
                        "ok": wrong_profile.get("ok"),
                        "code": wrong_profile.get("code"),
                        "operation": wrong_profile.get("operation"),
                        "error": wrong_profile.get("error"),
                    },
                    "stale_revision": {
                        "ok": stale_revision.get("ok"),
                        "code": stale_revision.get("code"),
                        "operation": stale_revision.get("operation"),
                        "error": stale_revision.get("error"),
                    },
                    "ambiguous_locator": {
                        "ok": ambiguous_locator.get("ok"),
                        "code": ambiguous_locator.get("code"),
                        "match_count": ambiguous_locator.get("match_count")
                        or ambiguous_locator.get("action_outcome", {}).get("match_count"),
                        "error": ambiguous_locator.get("error"),
                    },
                    "hidden_element": {
                        "ok": hidden_element.get("ok"),
                        "code": hidden_element.get("code"),
                        "extension_code": hidden_element.get("extension_code"),
                        "error": hidden_element.get("error"),
                    },
                    "disabled_element": {
                        "ok": disabled_element.get("ok"),
                        "code": disabled_element.get("code"),
                        "extension_code": disabled_element.get("extension_code"),
                        "error": disabled_element.get("error"),
                    },
                    "assertion_failed": {
                        "ok": assertion_failed.get("ok"),
                        "code": assertion_failed.get("code"),
                        "extension_code": assertion_failed.get("extension_code"),
                        "error": assertion_failed.get("error"),
                    },
                    "timed_out": {
                        "ok": timed_out.get("ok"),
                        "code": timed_out.get("code"),
                        "action_executed": timed_out.get("recovery", {}).get("action_executed"),
                        "retry": timed_out.get("recovery", {}).get("retry"),
                    },
                    "cross_profile_reference": {
                        "ok": cross_profile_reference.get("ok"),
                        "code": cross_profile_reference.get("code"),
                        "error": cross_profile_reference.get("error"),
                    },
                },
            )
            self.assertFalse(missing_element.get("ok"), missing_element)
            self.assertEqual(missing_element.get("code"), "element_not_found")
            self.assertEqual(missing_element.get("operation"), "execute_browser_action")
            self.assertEqual(missing_element.get("target"), target_a)
            self.assertEqual(wrong_lease.get("code"), "invalid_browser_lease")
            self.assertEqual(wrong_profile.get("code"), "browser_target_not_found")
            self.assertEqual(stale_revision.get("code"), "stale_browser_target")
            self.assertEqual(ambiguous_locator.get("code"), "stale_element_reference")
            self.assertEqual(
                ambiguous_locator.get("action_outcome", {}).get("match_count"),
                2,
            )
            self.assertFalse(hidden_element.get("ok"), hidden_element)
            self.assertEqual(hidden_element.get("code"), "browser_operation_failed")
            self.assertEqual(hidden_element.get("extension_code"), "not_visible")
            self.assertFalse(disabled_element.get("ok"), disabled_element)
            self.assertEqual(disabled_element.get("code"), "browser_operation_failed")
            self.assertEqual(disabled_element.get("extension_code"), "element_disabled")
            self.assertFalse(assertion_failed.get("ok"), assertion_failed)
            self.assertEqual(assertion_failed.get("code"), "browser_operation_failed")
            self.assertEqual(assertion_failed.get("extension_code"), "assert_text_failed")
            self.assertFalse(timed_out.get("ok"), timed_out)
            self.assertEqual(timed_out.get("code"), "browser_wait_timeout")
            self.assertTrue(timed_out.get("recovery", {}).get("action_executed"), timed_out)
            self.assertIn("do not retry", timed_out.get("recovery", {}).get("retry", ""))
            page_a = self.contexts["profile-a"].pages[0]
            self.assertEqual(await page_a.locator("#negative-click-count").text_content(), "0")
            self.assertEqual(await page_a.locator("#timeout-click-count").text_content(), "1")
            self.assertEqual(cross_profile_reference.get("code"), "stale_element_reference")
            self._log(
                "click_rejection_boundaries_verified",
                {
                    "missing_element": missing_element.get("code"),
                    "wrong_lease": wrong_lease.get("code"),
                    "wrong_profile": wrong_profile.get("code"),
                    "stale_revision": stale_revision.get("code"),
                    "ambiguous_locator": ambiguous_locator.get("code"),
                    "hidden_element": hidden_element.get("extension_code"),
                    "disabled_element": disabled_element.get("extension_code"),
                    "assertion_failed": assertion_failed.get("extension_code"),
                    "timed_out": timed_out.get("code"),
                    "cross_profile_reference": cross_profile_reference.get("code"),
                    "dispatch_retries": 0,
                    "timeout_action_count": "1",
                },
            )

            self._set_stage("execute CSS, test ID, role/name, and Snapshot refs through Rust")
            action_cases = []
            for profile, target, owner, token, revision in [
                ("profile-a", target_a, lease_owners[0], tokens[0], revision_a),
                ("profile-b", target_b, lease_owners[1], tokens[1], revision_b),
            ]:
                action_cases.extend(
                    [
                        (
                            f"rust-css-click-{profile}",
                            target,
                            owner,
                            token,
                            "click",
                            {"css": "#dom-click", "page_context_revision": revision},
                            "css",
                        ),
                        (
                            f"rust-test-id-click-{profile}",
                            target,
                            owner,
                            token,
                            "click",
                            {
                                "test_id": "test-id-click",
                                "page_context_revision": revision,
                            },
                            "test_id",
                        ),
                        (
                            f"rust-role-name-click-{profile}",
                            target,
                            owner,
                            token,
                            "click",
                            {
                                "role": "button",
                                "name": "Role click",
                                "page_context_revision": revision,
                            },
                            "role",
                        ),
                        (
                            f"rust-snapshot-pointer-{profile}",
                            target,
                            owner,
                            token,
                            "pointer_click",
                            {
                                "reference": snapshot_refs[target["extension_instance_id"]]["ref"],
                                "snapshot_id": snapshots[0 if profile == "profile-a" else 1][
                                    "snapshot_id"
                                ],
                                "page_context_revision": revision,
                            },
                            "snapshot_reference",
                        ),
                    ]
                )
            actions = await asyncio.gather(
                *(
                    self._control(
                        {
                            "schema_version": 1,
                            "request_id": request_id,
                            "caller_label": owner,
                            "project_root": str(self.temp_root),
                            "cmd": "execute_browser_action",
                            "target": target,
                            "lease_token": token,
                            "action": action,
                            "element": element,
                        }
                    )
                    for request_id, target, owner, token, action, element, _kind in action_cases
                )
            )
            for result, (request_id, target, _owner, _token, action, _element, _kind) in zip(
                actions, action_cases, strict=True
            ):
                self.assertTrue(result.get("ok"), result)
                self.assertEqual(result.get("request_id"), request_id)
                self.assertEqual(result.get("operation"), "execute_browser_action")
                self.assertEqual(result.get("extension_instance_id"), target["extension_instance_id"])
                self.assertEqual(result.get("target"), target)
                self.assertEqual(result.get("action_outcome", {}).get("action"), action)
            for index, action_case in enumerate(action_cases):
                if action_case[-1] == "snapshot_reference":
                    self.assertEqual(
                        actions[index].get("snapshot_id"),
                        snapshots[0 if action_case[1] == target_a else 1]["snapshot_id"],
                    )
                    snapshot_outcome = actions[index]["action_outcome"]
                    self.assertIsNone(snapshot_outcome.get("selector"))
                    self.assertEqual(snapshot_outcome["candidate"]["kind"], "role")
                    self.assertTrue(
                        snapshot_outcome["candidate"]["arguments"]["name"].startswith(
                            "Pointer click "
                        )
                    )
                elif action_case[-1] == "test_id":
                    self.assertEqual(
                        actions[index]["action_outcome"]["candidate"]["kind"],
                        "test_id",
                    )
                elif action_case[-1] == "role":
                    self.assertEqual(
                        actions[index]["action_outcome"]["candidate"]["kind"],
                        "role",
                    )
                else:
                    self.assertEqual(
                        actions[index]["action_outcome"].get("selector"),
                        "#dom-click",
                    )
                if action_case[4] == "pointer_click":
                    self.assertTrue(actions[index]["action_outcome"]["pointer"]["hit_verified"])
            self._log(
                "action_responses_verified",
                {
                    "responses": [
                        {
                            "request_id": result.get("request_id"),
                            "operation": result.get("operation"),
                            "extension_instance_id": result.get("extension_instance_id"),
                            "target": result.get("target"),
                            "action": result.get("action_outcome", {}).get("action"),
                            "locator_kind": action_cases[index][-1],
                            "pointer_hit_verified": result.get("action_outcome", {})
                            .get("pointer", {})
                            .get("hit_verified", False),
                        }
                        for index, result in enumerate(actions)
                    ]
                },
            )

            page_a = self.contexts["profile-a"].pages[0]
            page_b = self.contexts["profile-b"].pages[0]
            await asyncio.gather(
                page_a.wait_for_function(
                    "() => document.querySelector('#dom-click-count').textContent === '1' && document.querySelector('#test-id-click-count').textContent === '1' && document.querySelector('#role-click-count').textContent === '1' && document.querySelector('#pointer-click-count').textContent === '1' && document.querySelector('#pointerdown-count').textContent === '1'"
                ),
                page_b.wait_for_function(
                    "() => document.querySelector('#dom-click-count').textContent === '1' && document.querySelector('#test-id-click-count').textContent === '1' && document.querySelector('#role-click-count').textContent === '1' && document.querySelector('#pointer-click-count').textContent === '1' && document.querySelector('#pointerdown-count').textContent === '1'"
                ),
            )
            observed = {
                "profile-a": {
                    "dom_click": await page_a.locator("#dom-click-count").text_content(),
                    "test_id_click": await page_a.locator("#test-id-click-count").text_content(),
                    "role_click": await page_a.locator("#role-click-count").text_content(),
                    "pointer_click": await page_a.locator("#pointer-click-count").text_content(),
                    "pointerdown": await page_a.locator("#pointerdown-count").text_content(),
                },
                "profile-b": {
                    "dom_click": await page_b.locator("#dom-click-count").text_content(),
                    "test_id_click": await page_b.locator("#test-id-click-count").text_content(),
                    "role_click": await page_b.locator("#role-click-count").text_content(),
                    "pointer_click": await page_b.locator("#pointer-click-count").text_content(),
                    "pointerdown": await page_b.locator("#pointerdown-count").text_content(),
                },
            }
            self._log("dom_side_effects", observed)
            self.assertEqual(
                observed,
                {
                    "profile-a": {
                        "dom_click": "1",
                        "test_id_click": "1",
                        "role_click": "1",
                        "pointer_click": "1",
                        "pointerdown": "1",
                    },
                    "profile-b": {
                        "dom_click": "1",
                        "test_id_click": "1",
                        "role_click": "1",
                        "pointer_click": "1",
                        "pointerdown": "1",
                    },
                },
            )
            fresh_snapshot = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-snapshot-refresh-profile-a",
                    "caller_label": lease_owners[0],
                    "project_root": str(self.temp_root),
                    "cmd": "get_page_snapshot",
                    "target": target_a,
                    "lease_token": tokens[0],
                }
            )
            self.assertTrue(fresh_snapshot.get("ok"), fresh_snapshot)
            stale_reference = await self._control(
                {
                    "schema_version": 1,
                    "request_id": "rust-reference-stale-after-snapshot",
                    "caller_label": lease_owners[0],
                    "project_root": str(self.temp_root),
                    "cmd": "execute_browser_action",
                    "target": target_a,
                    "lease_token": tokens[0],
                    "action": "pointer_click",
                    "element": {
                        "reference": snapshot_refs[target_a["extension_instance_id"]]["ref"],
                        "snapshot_id": snapshots[0]["snapshot_id"],
                        "page_context_revision": revision_a,
                    },
                }
            )
            self.assertEqual(stale_reference.get("code"), "stale_element_reference")
            self.assertEqual(
                stale_reference.get("action_outcome"),
                None,
                stale_reference,
            )
            self._log(
                "click_pointer_acceptance_verified",
                {
                    "profiles": [target_a["extension_instance_id"], target_b["extension_instance_id"]],
                    "locator_kinds": ["css", "test_id", "role", "snapshot_reference"],
                    "click_count_per_profile": 3,
                    "pointer_click_count_per_profile": 1,
                    "pointerdown_count_per_profile": 1,
                    "stale_reference": stale_reference.get("code"),
                    "playwright_mutations": 0,
                },
            )
        finally:
            await asyncio.gather(
                self._release_lease(target_a, lease_owners[0], tokens[0]),
                self._release_lease(target_b, lease_owners[1], tokens[1]),
            )

        self._set_stage("disconnect Profile A and keep Profile B ready")
        context_a = self.contexts.pop("profile-a")
        worker_a = self.workers["profile-a"]
        try:
            await self._evaluate_worker(
                worker_a,
                """async () => {
                    await stopStreamSession();
                    closeStreamWebSocket();
                    return {streamState: streamWs ? streamWs.readyState : null};
                }""",
                timeout=5,
            )
        except Exception as error:  # noqa: BLE001
            self._log("profile_stream_stop_before_close_error", {"error": str(error)})
        await asyncio.wait_for(context_a.close(), timeout=10)
        self.workers.pop("profile-a", None)
        remaining = await self._wait_sessions(1, expected_count=1)
        ready_ids = {
            item["identity"]["extension_instance_id"] for item in remaining
        }
        self.assertEqual(ready_ids, {self.initial_ids["profile-b"]})
        self.assertIn(self.initial_ids["profile-b"], ready_ids)
        self._log(
            "profile_disconnected",
            {
                "remaining_ready_profiles": sorted(ready_ids),
                "profile_b_unchanged": self.initial_ids["profile-b"] in ready_ids,
            },
        )

        self._set_stage("reconnect Profile A with its persisted extension identity")
        await self._launch_context("profile-a", wait_for_worker=False)
        await asyncio.wait_for(
            self.contexts["profile-a"].pages[0].goto(
                f"http://127.0.0.1:{self.http.server_address[1]}/rust-a",
                wait_until="domcontentloaded",
            ),
            timeout=20,
        )
        # Chromium may restore the worker URL without exposing a new
        # Playwright Worker event/handle. Probe once for diagnostics, then use
        # the Broker's acknowledged stream generation as the live authority.
        reconnected_worker = await self._find_live_worker(self.contexts["profile-a"])
        if reconnected_worker is not None:
            self.workers["profile-a"] = reconnected_worker
        else:
            self._log(
                "service_worker_observation_gap",
                {
                    "profile": "profile-a",
                    "worker_urls": [
                        worker.url
                        for worker in self.contexts["profile-a"].service_workers
                    ],
                    "reason": "worker URL exists but no live Playwright handle/event",
                },
            )
        reconnected_session = await self._wait_session_stream_generation(
            self.initial_ids["profile-a"],
            greater_than=self.initial_stream_generations["profile-a"],
        )
        reconnected_a_generation = reconnected_session["stream_generation"]
        reconnected = await self._wait_sessions(2, expected_count=2)
        reconnected_a = self._session_for_path(reconnected, "/rust-a")
        reconnected_b = self._session_for_path(reconnected, "/rust-b")
        self.assertEqual(
            reconnected_a["identity"]["extension_instance_id"],
            self.initial_ids["profile-a"],
        )
        self.assertEqual(
            reconnected_b["identity"]["extension_instance_id"],
            self.initial_ids["profile-b"],
        )
        self._log(
            "profile_reconnect_recovered",
            {
                "profile_a_generation": reconnected_a_generation,
                "profile_a_identity": reconnected_a["identity"][
                    "extension_instance_id"
                ],
                "profile_b_identity": reconnected_b["identity"][
                    "extension_instance_id"
                ],
            },
        )

        self._set_stage("restart Rust Broker and reject old generation token")
        old_start_id = self.ready_endpoint["broker_start_id"]
        old_token = self.old_token
        old_port = self.discovery_port
        await self._stop_broker()
        await self._wait_port_closed(old_port)

        # Persistent Chrome Profiles may cache the service-worker script and
        # its endpoint constants. Reuse the same loopback port on restart so
        # recovery tests the broker generation change, not a script reload.
        self.discovery_port = old_port
        await self._start_broker()
        self.assertNotEqual(self.ready_endpoint["broker_start_id"], old_start_id)
        new_token = self._query_token(self.control_url)
        self._log(
            "broker_generation_rotated",
            {"token_changed": old_token != new_token},
        )
        old_token_url = urlunsplit(
            (
                "ws",
                urlsplit(self.control_url).netloc,
                "/",
                urlencode({"token": old_token}),
                "",
            )
        )
        with self.assertRaises(Exception):
            await self._control({}, url=old_token_url)
        invalid_token_url = urlunsplit(
            (
                "ws",
                urlsplit(self.control_url).netloc,
                "/",
                "token=definitely-invalid-test-token",
                "",
            )
        )
        with self.assertRaises(Exception):
            await self._control({}, url=invalid_token_url)
        self._log(
            "old_generation_rejected",
            {
                "old_generation_token_rejected": True,
                "new_broker_start_id": self.ready_endpoint["broker_start_id"],
                "old_token_recorded": False,
            },
        )

        await asyncio.gather(
            self._wait_session_stream_generation(self.initial_ids["profile-a"]),
            self._wait_session_stream_generation(self.initial_ids["profile-b"]),
        )
        recovered = await self._wait_sessions(2, expected_count=2)
        recovered_a = self._session_for_path(recovered, "/rust-a")
        recovered_b = self._session_for_path(recovered, "/rust-b")
        recovered_a_generation = recovered_a.get("stream_generation")
        recovered_b_generation = recovered_b.get("stream_generation")
        self.assertIsInstance(recovered_a_generation, int)
        self.assertIsInstance(recovered_b_generation, int)
        self.assertGreater(recovered_a_generation, 0)
        self.assertGreater(recovered_b_generation, 0)
        self.assertEqual(
            recovered_a["identity"]["extension_instance_id"],
            self.initial_ids["profile-a"],
        )
        self.assertEqual(
            recovered_b["identity"]["extension_instance_id"],
            self.initial_ids["profile-b"],
        )
        self._log(
            "broker_restart_recovered",
            {
                "profiles_ready": 2,
                "identities_preserved": True,
                "broker_features": self.ready_endpoint["broker_features"],
                "stream_generations": {
                    "profile-a": recovered_a_generation,
                    "profile-b": recovered_b_generation,
                },
            },
        )

    async def _wait_port_closed(self, port: int) -> None:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    pass
            except OSError:
                return
            await asyncio.sleep(0.1)
        raise AssertionError(f"Rust Broker discovery port remained open: {port}")

    def _rewrite_extension_port(self, port: int) -> None:
        path = self.extension_copy / "background.js"
        text = path.read_text(encoding="utf-8")
        text, replacements = re.subn(
            r"http://127\.0\.0\.1:\d+",
            f"http://127.0.0.1:{port}",
            text,
        )
        if replacements != 3:
            raise AssertionError(
                f"expected three Broker URLs in copied extension, found {replacements}"
            )
        path.write_text(text, encoding="utf-8")

    def _http_discovery(self, origin: str | None, method: str = "GET") -> dict:
        url = f"http://127.0.0.1:{self.discovery_port}/v1/bridge"
        request = Request(
            url,
            data=b"{}" if method == "POST" else None,
            method=method,
        )
        if method == "POST":
            request.add_header("Content-Type", "application/json")
        if origin is not None:
            request.add_header("Origin", origin)
        try:
            with urlopen(request, timeout=5) as response:
                payload = json.loads(response.read().decode("utf-8"))
                return {"status": response.status, "payload": payload}
        except HTTPError as error:
            return {"status": error.code, "payload": {}}
        except URLError as error:
            raise AssertionError(f"Rust Broker discovery failed: {error}") from error

    @staticmethod
    def _target(session: dict) -> dict:
        tab = next(
            tab
            for window in session["windows"]
            for tab in window["tabs"]
            if tab.get("active") and tab.get("debuggable")
        )
        return {
            "extension_instance_id": session["identity"]["extension_instance_id"],
            "window_id": tab["window_id"],
            "tab_id": tab["id"],
        }

    @staticmethod
    def _query_token(url: str) -> str:
        return parse_qs(urlsplit(url).query).get("token", [""])[0]

    @staticmethod
    def _worker_origin(worker: Worker) -> str:
        parsed = urlsplit(worker.url)
        return urlunsplit((parsed.scheme, parsed.netloc, "", "", ""))

    @staticmethod
    def _unused_port() -> int:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            return int(sock.getsockname()[1])

    @staticmethod
    def _version(*command: str) -> str:
        try:
            result = subprocess.run(
                list(command), capture_output=True, text=True, timeout=10, check=False
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            return f"unavailable: {error}"
        return (result.stdout or result.stderr).strip()

    @staticmethod
    def _working_set_bytes(pid: int | None) -> int | None:
        if pid is None or pid <= 0:
            return None
        if os.name != "nt":
            return None
        result = subprocess.run(
            [
                "powershell",
                "-NoProfile",
                "-Command",
                f"(Get-Process -Id {int(pid)} -ErrorAction SilentlyContinue).WorkingSet64",
            ],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
        try:
            value = int(result.stdout.strip())
        except (TypeError, ValueError):
            return None
        return value if value > 0 else None

    @staticmethod
    def _redact(value: str) -> str:
        return SECRET_QUERY.sub(r"\1<redacted>", value)

    @staticmethod
    def _safe_sessions(sessions: list[dict]) -> list[dict]:
        return [
            {
                "extension_instance_id": item.get("identity", {}).get(
                    "extension_instance_id"
                ),
                "health": item.get("health"),
                "stream_generation": item.get("stream_generation"),
                "urls": [
                    tab.get("url")
                    for window in item.get("windows", [])
                    for tab in window.get("tabs", [])
                ],
            }
            for item in sessions
        ]

    def _set_stage(self, stage: str) -> None:
        self.stage = stage
        self._log("stage_start", {"stage": stage})

    def _log(self, event: str, payload: dict) -> None:
        if self.diagnostics_path is None:
            return
        record = {
            "time": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            "monotonic": round(time.monotonic(), 3),
            "stage": getattr(self, "stage", "setup"),
            "event": event,
            "payload": json.loads(self._redact(json.dumps(payload, ensure_ascii=False))),
        }
        with self.diagnostics_path.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(record, ensure_ascii=False, sort_keys=True) + "\n")


if __name__ == "__main__":
    unittest.main()
