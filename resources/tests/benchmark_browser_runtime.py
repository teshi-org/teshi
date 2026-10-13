"""Opt-in real Chromium benchmark with identical Python/Rust wire workloads.

Run with TESHI_CLI pointing at a release binary. This is a local measurement,
not a CI latency gate. It uses only temporary Profiles, ports, and state.
"""

from __future__ import annotations

import argparse
import asyncio
import ctypes
import hashlib
import importlib.metadata
import json
import math
import os
import platform
import statistics
import subprocess
import sys
import threading
import time
import uuid
from pathlib import Path

import websockets

from test_browser_two_profile_rust_transport import (
    REPO_ROOT,
    TESHI_CLI,
    BrowserTwoProfileRustTransportTests,
)


class MemorySampler:
    """Sample the owned Windows process handle, including its OS peak RSS.

    Keeping the handle avoids PID reuse. Browser and driver memory are excluded.
    Other platforms deliberately fail instead of reporting incomparable zeros.
    """

    class Counters(ctypes.Structure):
        _fields_ = [
            ("cb", ctypes.c_ulong), ("PageFaultCount", ctypes.c_ulong),
            ("PeakWorkingSetSize", ctypes.c_size_t),
            ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
            ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t),
            ("PeakPagefileUsage", ctypes.c_size_t),
            ("PrivateUsage", ctypes.c_size_t),
        ]

    def __init__(self, process: subprocess.Popen) -> None:
        if os.name != "nt":
            raise RuntimeError("This memory benchmark currently requires Windows")
        self.handle = int(process._handle)
        self.query = ctypes.WinDLL("psapi", use_last_error=True).GetProcessMemoryInfo
        self.query.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_ulong]
        self.query.restype = ctypes.c_int
        self.stop_event = threading.Event()
        self.phase = "idle"
        self.samples: list[dict] = []
        self.error: str | None = None
        self.thread = threading.Thread(target=self._run, daemon=True)

    def _run(self) -> None:
        while not self.stop_event.is_set():
            counters = self.Counters()
            counters.cb = ctypes.sizeof(counters)
            if not self.query(self.handle, ctypes.byref(counters), counters.cb):
                self.error = f"GetProcessMemoryInfo failed: {ctypes.get_last_error()}"
                return
            self.samples.append({
                "phase": self.phase,
                "elapsed_ms": round((time.perf_counter() - self.started) * 1000, 3),
                "working_set_bytes": counters.WorkingSetSize,
                "private_bytes": counters.PrivateUsage,
                "os_peak_working_set_bytes": counters.PeakWorkingSetSize,
            })
            self.stop_event.wait(0.02)

    def start(self) -> None:
        self.started = time.perf_counter()
        self.thread.start()

    def finish(self) -> dict:
        self.stop_event.set()
        self.thread.join(timeout=2)
        if self.thread.is_alive() or self.error or not self.samples:
            raise RuntimeError(self.error or "Memory sampler failed to finish")
        idle = [s["working_set_bytes"] for s in self.samples if s["phase"] == "idle"]
        return {
            "interval_ms": 20,
            "steady_working_set_bytes": statistics.median(idle),
            "sampled_peak_working_set_bytes": max(s["working_set_bytes"] for s in self.samples),
            "os_peak_working_set_bytes": max(s["os_peak_working_set_bytes"] for s in self.samples),
            "sampled_peak_private_bytes": max(s["private_bytes"] for s in self.samples),
            "samples": self.samples,
        }


class RuntimeFixture(BrowserTwoProfileRustTransportTests):
    """Reuse the acceptance fixture's temporary resources and owned cleanup."""

    async def _wait_broker_stdout_line(self, timeout: float = 15) -> str:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if not self.broker_stdout_lines.empty():
                return self.broker_stdout_lines.get_nowait()
            if self.broker.poll() is not None:
                break
            await asyncio.sleep(0.01)
        raise AssertionError("Owned benchmark broker failed to announce readiness")

    async def _start_broker(self) -> None:
        self.startup_ms = []
        for index in range(3):
            started = time.perf_counter()
            if self.implementation == "rust":
                await super()._start_broker()
            else:
                # Windows venv python.exe is a redirector with a child process.
                # Run its base interpreter with the driver's library paths so
                # the held Popen handle measures the actual broker, not the stub.
                python_env = os.environ.copy()
                python_env["PYTHONPATH"] = os.pathsep.join(sys.path)
                self.broker = subprocess.Popen([
                    sys._base_executable, "-B", str(REPO_ROOT / "resources/browser_service.py"),
                    "--host", "127.0.0.1", "--port", "0", "--mode", "chrome",
                    "--discovery-port", str(self.discovery_port),
                    "--project-root", str(self.temp_root),
                ], cwd=REPO_ROOT, env=python_env, stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                self.broker_stdout_thread = threading.Thread(
                    target=self._drain_broker_stdout, args=(self.broker.stdout,), daemon=True)
                self.broker_stderr_thread = threading.Thread(
                    target=self._drain_broker_stderr, args=(self.broker.stderr,), daemon=True)
                self.broker_stdout_thread.start()
                self.broker_stderr_thread.start()
                line = await self._wait_broker_stdout_line()
                self.assertTrue(line.strip().isdigit(), "Python readiness port missing")
                endpoint = json.loads((self.temp_root / ".teshi/cdp-endpoint.json").read_text())
                self.assertEqual(endpoint["broker_pid"], self.broker.pid)
                self.control_url = endpoint["ws_url"]
            # Same preflight proves readiness rather than measuring spawn alone.
            self.assertTrue((await self._list_sessions()).get("ok"))
            self.startup_ms.append(round((time.perf_counter() - started) * 1000, 3))
            if index < 2:
                await self._stop_broker()
                await self._wait_port_closed(self.discovery_port)

    async def _control(self, payload: dict, *, url: str | None = None, timeout: float = 20) -> dict:
        # Python can send an initial preview on this route. Correlate responses
        # for both implementations; neither incidental frames nor retries count.
        started = time.perf_counter()
        async with websockets.connect(
            url or self.control_url, origin=self.extension_origin,
            open_timeout=5, close_timeout=2, max_size=2 * 1024 * 1024,
        ) as socket:
            await socket.send(json.dumps(payload))
            async with asyncio.timeout(timeout):
                while True:
                    message = await socket.recv()
                    if isinstance(message, bytes):
                        continue
                    response = json.loads(message)
                    if response.get("request_id") == payload["request_id"]:
                        response_ms = (time.perf_counter() - started) * 1000
                        break
        self.last_response_ms = round(response_ms, 3)
        self.last_close_ms = round((time.perf_counter() - started) * 1000 - response_ms, 3)
        return response

    async def _acquire_lease(self, target: dict, owner: str) -> str:
        response = await self._control({
            "schema_version": 1, "request_id": uuid.uuid4().hex,
            "caller_label": owner, "project_root": str(self.temp_root),
            "cmd": "acquire_browser_lease", "extension_instance_id": target["extension_instance_id"],
            "owner_label": owner, "ttl_secs": 300,
        })
        self.assertTrue(response.get("ok"), self._redact(json.dumps(response)))
        token = response.get("lease_token") or response.get("lease", {}).get("lease_token")
        self.assertIsInstance(token, str)
        return token

    async def measure(self, samples: int, burst: int) -> dict:
        sessions = await self._wait_sessions(2, expected_count=2)
        targets = [self._target(self._session_for_path(sessions, path)) for path in ("/rust-a", "/rust-b")]
        owners = ["runtime-benchmark-a", "runtime-benchmark-b"]
        tokens = [await self._acquire_lease(target, owner) for target, owner in zip(targets, owners)]
        operations: dict[str, list[float]] = {}
        responses: dict[str, list[float]] = {}
        closes: dict[str, list[float]] = {}
        screenshot_sizes: dict[str, list[int]] = {"png": [], "jpeg": []}
        sampler = MemorySampler(self.broker)
        sampler.start()
        network_active = False

        async def request(index: int, command: str, label: str | None = None, **fields: object) -> dict:
            payload = {
                "schema_version": 1, "request_id": uuid.uuid4().hex,
                "caller_label": owners[index], "project_root": str(self.temp_root),
                "cmd": command, "target": targets[index], "lease_token": tokens[index], **fields,
            }
            started = time.perf_counter()
            response = await self._control(payload)
            elapsed = round((time.perf_counter() - started) * 1000, 3)
            self.assertTrue(response.get("ok"), self._redact(json.dumps(response)))
            if label is not None:
                operations.setdefault(label, []).append(elapsed)
                responses.setdefault(label, []).append(self.last_response_ms)
                closes.setdefault(label, []).append(self.last_close_ms)
            return response

        try:
            await asyncio.sleep(2)
            sampler.phase = "operations"
            # Two warmups per operation; identical mutations remain verified.
            for iteration in range(samples + 2):
                measured = iteration >= 2
                index = iteration % 2
                label = lambda name: name if measured else None
                page = self.contexts[f"profile-{'ab'[index]}"].pages[0]
                snapshot = await request(index, "get_page_snapshot", label("snapshot"))
                self.assertEqual(snapshot["url"], page.url)
                locator = await request(index, "resolve_playwright_locator", label("locator"),
                                        intent={"role": "textbox", "text": "Project value"})
                self.assertEqual(locator["element"]["id"], "project-value")
                revision = snapshot["page_context_revision"]
                value = f"benchmark-{iteration}"
                await request(index, "execute_browser_action", label("fill"), action="fill", value=value,
                              element={"css": "#project-value", "page_context_revision": revision})
                await request(index, "execute_browser_action", label("assert"), action="assert_text", value=value,
                              element={"css": "#project-echo", "page_context_revision": revision})
                await request(index, "execute_browser_action", label("click"), action="click",
                              element={"css": "#dom-click", "page_context_revision": revision})
                self.assertEqual(await page.locator("#project-value").input_value(), value)
                self.assertEqual(await page.locator("#dom-click-count").text_content(), str(iteration // 2 + 1))
                for image_format in ("png", "jpeg"):
                    fields = {"format": image_format, "full_page": image_format == "jpeg"}
                    if image_format == "jpeg":
                        fields["quality"] = 75
                    response = await request(index, "capture_browser_screenshot", label(f"screenshot_{image_format}"), **fields)
                    path = self.temp_root / ".teshi/artifacts/browser" / response["artifact"]["path"]
                    data = path.read_bytes()
                    self.assertTrue(data.startswith(b"\x89PNG\r\n\x1a\n" if image_format == "png" else b"\xff\xd8"))
                    if measured:
                        screenshot_sizes[image_format].append(len(data))
            # Navigation changes the heartbeat's target revision. Keep it in a
            # separate phase and observe published readiness before proceeding;
            # otherwise a late heartbeat can correctly invalidate a snapshot.
            for iteration in range(samples + 2):
                index = iteration % 2
                page = self.contexts[f"profile-{'ab'[index]}"].pages[0]
                url = f"http://127.0.0.1:{self.http.server_address[1]}/benchmark-{index}-{iteration}"
                await request(index, "navigate", "navigate" if iteration >= 2 else None,
                              url=url, timeout_ms=15_000)
                await page.wait_for_url(url)
                await page.wait_for_load_state("domcontentloaded")
                await self._wait_target_url(targets[index], url)
            sampler.phase = "network"
            await request(1, "start_network_capture", allowed_hostnames=["127.0.0.1"],
                          max_entries=128, max_bytes=128 * 1024, max_body_bytes=1024,
                          capture_request_bodies=False)
            network_active = True
            started = time.perf_counter()
            result = await self.contexts["profile-b"].pages[0].evaluate(
                """async ({port, count}) => {
                    let completed = 0;
                    for (let offset = 0; offset < count; offset += 25) {
                        await Promise.all(Array.from({length: Math.min(25, count-offset)}, async (_, i) => {
                            const response = await fetch(`http://127.0.0.1:${port}/benchmark-network-${offset+i}`, {method:'POST',body:'synthetic'});
                            if (!response.ok) throw new Error('benchmark HTTP failure');
                            await response.text(); completed++;
                        }));
                    }
                    return {completed};
                }""", {"port": self.http.server_address[1], "count": burst})
            fetch_ms = round((time.perf_counter() - started) * 1000, 3)
            self.assertEqual(result["completed"], burst)
            # A saturated bounded queue can legitimately drop the final HTTP
            # request's events. Measure loss rather than require that request
            # to survive retention; require a contiguous ack and a quiet queue.
            deadline = time.monotonic() + 20
            stable_since = time.monotonic()
            previous_delivery = None
            while True:
                network = await request(1, "list_network_requests")
                delivery = await self._evaluate_worker(self.workers["profile-b"],
                    "Array.from(networkDeliveryStates.values()).map(s=>({queue_length:s.queue.length,queue_bytes:s.queue_bytes,dropped_events:s.dropped_events_total,dropped_bytes:s.dropped_bytes_total,next_seq:s.next_seq,acked_seq:s.acked_seq,active:s.active}))", timeout=3)
                # Persist bounded diagnostics even when this measurement fails.
                (self.artifact_dir / "network-progress.json").write_text(json.dumps({
                    "retained_entries": network.get("retained_entries"),
                    "retained_bytes": network.get("retained_bytes"),
                    "broker_delivery": network.get("delivery", {}),
                    "termination": network.get("termination", {}),
                    "extension_delivery": delivery,
                    "request_indices": [int(item["url"].split("/benchmark-network-")[-1])
                                        for item in network["requests"] if "/benchmark-network-" in item.get("url", "")],
                }, indent=2) + "\n", encoding="utf-8")
                if delivery != previous_delivery:
                    stable_since = time.monotonic()
                    previous_delivery = delivery
                if network["requests"] and delivery and all(
                    s["queue_length"] == 0 and s["acked_seq"] == s["next_seq"] - 1
                    for s in delivery
                ) and time.monotonic() - stable_since >= 1:
                    break
                self.assertLess(time.monotonic(), deadline, "Network burst failed to settle")
                await asyncio.sleep(0.1)
            self.assertLessEqual(network["retained_entries"], 128)
            self.assertLessEqual(network["retained_bytes"], 128 * 1024)
            self.assertTrue(all("request_body" not in item for item in network["requests"]))
            self.assertTrue(all(s["active"] for s in delivery), delivery)
            self.assertTrue(all(s["queue_bytes"] == 0 for s in delivery), delivery)
            broker_delivery = network.get("delivery", {})
            self.assertEqual(broker_delivery.get("pending_sequences"), 0)
            self.assertEqual(broker_delivery.get("rejected_events"), 0)
            self.assertEqual(broker_delivery.get("acknowledged_sequence"), delivery[0]["acked_seq"])
            network_metrics = {
                "requests": burst, "concurrency": 25, "fetch_complete_ms": fetch_ms,
                "delivery_settled_ms": round((time.perf_counter() - started) * 1000, 3),
                "retained_entries": network["retained_entries"], "retained_bytes": network["retained_bytes"],
                "broker_delivery": network.get("delivery", {}), "extension_delivery": delivery,
            }
            await request(1, "stop_network_capture")
            network_active = False
            sampler.phase = "after-network"
            await asyncio.sleep(1)
            return {"startup_ms": self.startup_ms, "operations_ms": operations,
                    "response_ms": responses, "close_ms": closes,
                    "screenshot_sizes": screenshot_sizes, "network": network_metrics,
                    "memory": sampler.finish()}
        finally:
            if sampler.thread.is_alive():
                sampler.finish()
            if network_active:
                await request(1, "stop_network_capture")
            for target, owner, token in zip(targets, owners, tokens):
                await self._release_lease(target, owner, token)


def distribution(values: list[float]) -> dict:
    ordered = sorted(values)
    return {"n": len(values), "median": round(statistics.median(values), 3),
            "p95_nearest_rank": round(ordered[math.ceil(len(values) * 0.95) - 1], 3),
            "min": min(values), "max": max(values)}


async def run(args: argparse.Namespace) -> None:
    args.output.mkdir(parents=True, exist_ok=False)
    report = {
        "scope": "Windows; release Rust broker vs current Python oracle; same direct WebSocket requests including connect/close; synthetic local pages; no CLI spawn in operation timings",
        "startup_scope": "new process through ready announcement and empty-session WebSocket preflight; fresh Python process, reused Rust private state; warm OS file caches; not production launcher latency",
        "environment": {"platform": platform.platform(), "python": sys.version,
                        "python_broker_executable": sys._base_executable,
                        "websockets": importlib.metadata.version("websockets"),
                        "playwright": importlib.metadata.version("playwright"),
                        "benchmark_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                        "extension_background_sha256": hashlib.sha256((REPO_ROOT / "extension/teshi-bridge/background.js").read_bytes()).hexdigest(),
                        "python_service_sha256": hashlib.sha256((REPO_ROOT / "resources/browser_service.py").read_bytes()).hexdigest(),
                        "cli": str(TESHI_CLI), "cli_sha256": hashlib.sha256(TESHI_CLI.read_bytes()).hexdigest(),
                        "cli_version": subprocess.check_output([str(TESHI_CLI), "--version"], text=True).strip(),
                        "source_head": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()},
        "rounds": [],
    }
    if args.recorded_python_baseline:
        baseline = next(json.loads(line) for line in args.recorded_python_baseline.read_text(encoding="utf-8").splitlines()
                        if json.loads(line).get("event") == "stage5_6_python_baseline")
        report["historical_python_baseline"] = {
            "path": str(args.recorded_python_baseline), "time": baseline["time"],
            "sha256": hashlib.sha256(args.recorded_python_baseline.read_bytes()).hexdigest(),
            "metrics": baseline["payload"],
            "comparability": "Single old CLI-path sample; no startup or locator data; sampled peak only. Context, not a direct ratio to this same-wire benchmark.",
        }
    for round_index in range(args.rounds):
        # Alternating first implementation reduces a consistent warm-cache bias.
        order = ("python", "rust") if round_index % 2 == 0 else ("rust", "python")
        for implementation in order:
            artifact = args.output / f"round-{round_index + 1}-{implementation}"
            os.environ["TESHI_RUST_TRANSPORT_ARTIFACT_DIR"] = str(artifact.resolve())
            fixture = RuntimeFixture("runTest")
            fixture.implementation = implementation
            try:
                await fixture.asyncSetUp()
                metrics = await fixture.measure(args.samples, args.burst)
                report["rounds"].append({"round": round_index + 1, "implementation": implementation,
                                       "chromium": fixture.contexts["profile-a"].browser.version, **metrics})
                print(f"Completed round {round_index + 1}: {implementation}", flush=True)
            finally:
                await fixture._cleanup()
                (args.output / "measurements.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    report["summary"] = {}
    for implementation in ("python", "rust"):
        rounds = [r for r in report["rounds"] if r["implementation"] == implementation]
        report["summary"][implementation] = {
            "startup_ms": distribution([v for r in rounds for v in r["startup_ms"]]),
            "operations_ms": {key: distribution([v for r in rounds for v in r["operations_ms"][key]]) for key in rounds[0]["operations_ms"]},
            "response_ms": {key: distribution([v for r in rounds for v in r["response_ms"][key]]) for key in rounds[0]["response_ms"]},
            "close_ms": {key: distribution([v for r in rounds for v in r["close_ms"][key]]) for key in rounds[0]["close_ms"]},
            "steady_working_set_bytes": distribution([r["memory"]["steady_working_set_bytes"] for r in rounds]),
            "os_peak_working_set_bytes": distribution([r["memory"]["os_peak_working_set_bytes"] for r in rounds]),
            "network_delivery_settled_ms": distribution([r["network"]["delivery_settled_ms"] for r in rounds]),
        }
    (args.output / "measurements.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report["summary"], indent=2), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="New evidence directory")
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--samples", type=int, default=10)
    parser.add_argument("--burst", type=int, default=500)
    parser.add_argument("--recorded-python-baseline", type=Path)
    options = parser.parse_args()
    if not (1 <= options.rounds <= 10 and 2 <= options.samples <= 100 and 1 <= options.burst <= 5000):
        parser.error("rounds=1..10, samples=2..100, burst=1..5000 required")
    asyncio.run(run(options))
