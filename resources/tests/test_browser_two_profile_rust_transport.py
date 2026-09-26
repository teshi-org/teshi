"""Real Chromium two-Profile transport acceptance for the Rust Broker."""

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
        profile = self.path.split("?", 1)[0].strip("/") or "unknown"
        body = (
            f"<!doctype html><meta charset=utf-8><title>Rust {profile}</title>"
            f"<main data-profile=\"{profile}\">{profile}</main>"
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
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
        self.assertEqual(self.ready_endpoint["broker_features"], ["transport.v1"])
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

    async def _launch_context(self, name: str, *, keep: bool = True) -> BrowserContext:
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
                worker = await self._wait_worker(context)
            except Exception:
                await asyncio.wait_for(context.close(), timeout=10)
                raise
            self.contexts[name] = context
            self.workers[name] = worker
        return context

    async def _wait_worker(self, context: BrowserContext) -> Worker:
        if context.service_workers:
            return context.service_workers[0]
        try:
            return await context.wait_for_event("serviceworker", timeout=15_000)
        except Exception as error:  # noqa: BLE001
            raise AssertionError(
                f"Chrome extension service worker did not start: {error}"
            ) from error

    async def _refresh_worker(self, name: str) -> Worker:
        """Use the current worker after a persistent Profile restart."""
        context = self.contexts[name]
        for worker in reversed(context.service_workers):
            try:
                await self._evaluate_worker(worker, "true", timeout=3)
            except Exception:
                continue
            self.workers[name] = worker
            self._log("service_worker_selected", {"profile": name, "url": worker.url})
            return worker
        try:
            worker = await context.wait_for_event("serviceworker", timeout=15_000)
            await self._evaluate_worker(worker, "true", timeout=3)
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

    async def _wait_stream(self, name: str) -> None:
        worker = self.workers[name]
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            try:
                connected = await self._evaluate_worker(
                    worker,
                    "typeof streamWs !== 'undefined' && "
                    "streamWs !== null && streamWs.readyState === WebSocket.OPEN",
                    timeout=2,
                )
            except asyncio.TimeoutError:
                connected = False
            except Exception:
                connected = False
            if connected:
                self._log("stream_hello_transport_ready", {"profile": name})
                return
            await asyncio.sleep(0.25)
        try:
            diagnostics = await self._evaluate_worker(
                worker,
                "({bridge: lastBridgeStatus, projectNeutral: cachedBrokerProjectNeutral, "
                "projectRoot: cachedProjectRoot, frameWs: Boolean(extensionFrameWsUrl), "
                "streamState: streamWs ? streamWs.readyState : null, "
                "screencastActive, streamSessionTabId})",
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
                    "streamState: streamWs ? streamWs.readyState : null, screencastActive})",
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

    async def _control(self, payload: dict, *, url: str | None = None) -> dict:
        control_url = url or self.control_url
        async with websockets.connect(
            control_url,
            origin=self.extension_origin,
            open_timeout=5,
            close_timeout=2,
            max_size=2 * 1024 * 1024,
        ) as socket:
            await socket.send(json.dumps(payload))
            message = await asyncio.wait_for(socket.recv(), timeout=5)
        if isinstance(message, bytes):
            message = message.decode("utf-8")
        return json.loads(message)

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
        self.assertEqual(trusted["broker_features"], ["transport.v1"])
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
        await asyncio.gather(self._wait_stream("profile-a"), self._wait_stream("profile-b"))
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

        self._set_stage("verify target ambiguity and transport-only control gate")
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
        gated = await self._control(
            {
                "schema_version": 1,
                "request_id": "rust-p0-gate",
                "caller_label": "rust-transport-test",
                "project_root": str(self.temp_root),
                "cmd": "get_page_snapshot",
                "target": target_a,
            }
        )
        self.assertEqual(gated["code"], "browser_capability_unavailable")
        self._log(
            "control_boundary_verified",
            {
                "implicit_target_error": ambiguous["code"],
                "explicit_p0_error": gated["code"],
                "navigation_executed": False,
            },
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
        await self._launch_context("profile-a")
        await asyncio.wait_for(
            self.contexts["profile-a"].pages[0].goto(
                f"http://127.0.0.1:{self.http.server_address[1]}/rust-a",
                wait_until="domcontentloaded",
            ),
            timeout=20,
        )
        await self._refresh_worker("profile-a")
        await self._force_stream("profile-a")
        await self._wait_stream("profile-a")
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
            self._force_stream("profile-a"),
            self._force_stream("profile-b"),
        )
        await asyncio.gather(self._wait_stream("profile-a"), self._wait_stream("profile-b"))
        recovered = await self._wait_sessions(2, expected_count=2)
        recovered_a = self._session_for_path(recovered, "/rust-a")
        recovered_b = self._session_for_path(recovered, "/rust-b")
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
