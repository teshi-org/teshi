"""Real two-Profile Python Chrome evidence baseline for Rust stage 5.6."""

from __future__ import annotations

import asyncio
import contextlib
import json
import os
import shutil
import subprocess
import time
import unittest
from pathlib import Path

from test_browser_two_profile_p0 import BrowserTwoProfileP0Tests


class BrowserTwoProfilePythonEvidenceTests(BrowserTwoProfileP0Tests):
    @unittest.skip("P0 control loop is covered by test_browser_two_profile_p0.py")
    async def test_two_profiles_execute_concurrently_without_cross_routing(self) -> None:
        return

    @staticmethod
    def _working_set_bytes(pid: int | None) -> int | None:
        if pid is None or pid <= 0 or os.name != "nt":
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

    async def test_python_evidence_latency_and_memory_baseline(self) -> None:
        self._set_stage("stage 5.6 Python screenshot, Console, and Network baseline")
        sessions = await self.wait_for_new_profiles(self.baseline)
        sessions.sort(key=lambda item: item["identity"]["extension_instance_id"])
        targets = [self.active_target(session) for session in sessions]
        leases = await asyncio.gather(
            self.cli(
                "lease",
                "acquire",
                "--session",
                targets[0]["session"],
                "--owner",
                "python-stage5-6-profile-a",
            ),
            self.cli(
                "lease",
                "acquire",
                "--session",
                targets[1]["session"],
                "--owner",
                "python-stage5-6-profile-b",
            ),
        )
        tokens = [lease["lease"]["lease_token"] for lease in leases]
        metrics: dict[str, object] = {
            "operations_ms": {},
            "working_set_bytes": [],
        }
        network_started = False

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

        async def timed(label: str, *args: str, timeout: float = 30) -> dict:
            started = time.perf_counter()
            result = await self.cli(*args, timeout=timeout)
            operations = metrics["operations_ms"]
            assert isinstance(operations, dict)
            operations[label] = round((time.perf_counter() - started) * 1000, 1)
            sample_working_set(label)
            return result

        try:
            sample_working_set("before-evidence")
            screenshots = await asyncio.gather(
                timed(
                    "screenshot_png_profile_a",
                    "screenshot",
                    "--format",
                    "png",
                    *self.target_args(targets[0], tokens[0]),
                ),
                timed(
                    "screenshot_jpeg_profile_b",
                    "screenshot",
                    "--format",
                    "jpeg",
                    "--quality",
                    "75",
                    "--full-page",
                    *self.target_args(targets[1], tokens[1]),
                ),
            )
            screenshot_paths = [Path(result["artifact"]["path"]) for result in screenshots]
            self.assertEqual(screenshot_paths[0].read_bytes()[:8], b"\x89PNG\r\n\x1a\n")
            self.assertEqual(screenshot_paths[1].read_bytes()[:2], b"\xff\xd8")
            if self.artifact_dir is not None:
                self.artifact_dir.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(
                    screenshot_paths[0],
                    self.artifact_dir / "python-stage5-6-profile-a.png"
                )
                shutil.copyfile(
                    screenshot_paths[1],
                    self.artifact_dir / "python-stage5-6-profile-b.jpg"
                )

            port = self.http.server_address[1]
            await asyncio.gather(
                timed(
                    "navigate_profile_a",
                    "navigate",
                    f"http://127.0.0.1:{port}/python-a",
                    *self.target_args(targets[0], tokens[0]),
                ),
                timed(
                    "navigate_profile_b",
                    "navigate",
                    f"http://127.0.0.1:{port}/python-b",
                    *self.target_args(targets[1], tokens[1]),
                ),
            )
            await asyncio.gather(
                self.wait_for_target_url(targets[0], f"http://127.0.0.1:{port}/python-a"),
                self.wait_for_target_url(targets[1], f"http://127.0.0.1:{port}/python-b"),
            )
            pages = {}
            for context in self.contexts:
                page = context.pages[0]
                if "/python-a" in page.url:
                    pages["a"] = page
                elif "/python-b" in page.url:
                    pages["b"] = page
            self.assertEqual(set(pages), {"a", "b"}, pages)

            console_start = await timed(
                "console_start_profile_a",
                "console",
                "start",
                "--level",
                "info,error",
                "--max-entries",
                "8",
                "--max-bytes",
                "4096",
                "--max-age-ms",
                "60000",
                "--sensitive-field",
                "token",
                *self.target_args(targets[0], tokens[0]),
            )
            self.assertTrue(console_start.get("ok"), console_start)
            await pages["a"].evaluate(
                """() => {
                    console.info('stage5 token=python-console-secret');
                    console.error('stage5-large-' + 'x'.repeat(70000));
                }"""
            )
            console_list: dict = {}
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                console_list = await timed(
                    "console_list_profile_a",
                    "console",
                    "list",
                    *self.target_args(targets[0], tokens[0]),
                )
                if console_list.get("events"):
                    break
                await asyncio.sleep(0.25)
            self.assertTrue(console_list.get("events"), console_list)
            self.assertNotIn("python-console-secret", json.dumps(console_list))
            self.assertLessEqual(console_list.get("retained_bytes", 0), 4096)
            self.assertTrue(
                any(event.get("truncated") for event in console_list["events"])
            )
            console_stop = await timed(
                "console_stop_profile_a",
                "console",
                "stop",
                *self.target_args(targets[0], tokens[0]),
            )
            self.assertTrue(console_stop.get("ok"), console_stop)

            network_start = await timed(
                "network_start_profile_b",
                "network",
                "start",
                "--host",
                "127.0.0.1",
                "--request-body",
                "--max-request-body-bytes",
                "64",
                "--max-body-bytes",
                "1024",
                "--max-entries",
                "256",
                "--max-bytes",
                str(256 * 1024),
                "--sensitive-field",
                "token",
                *self.target_args(targets[1], tokens[1]),
            )
            self.assertTrue(network_start.get("ok"), network_start)
            network_started = True
            await pages["b"].evaluate(
                """async ({port}) => {
                    const response = await fetch(
                        `http://127.0.0.1:${port}/python-network`,
                    );
                    await response.text();
                    await fetch(`http://localhost:${port}/python-filtered`).catch(() => null);
                }""",
                {"port": port},
            )
            network_list: dict = {}
            matching_request: dict | None = None
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                network_list = await timed(
                    "network_list_profile_b",
                    "network",
                    "list",
                    *self.target_args(targets[1], tokens[1]),
                )
                matching_request = next(
                    (
                        item
                        for item in network_list.get("requests", [])
                        if "/python-network" in str(item.get("url", ""))
                    ),
                    None,
                )
                if matching_request is not None:
                    break
                await asyncio.sleep(0.25)
            self.assertIsNotNone(matching_request, network_list)
            assert matching_request is not None
            self.assertNotIn("request_body", matching_request)
            self.assertNotIn("python-network-secret", json.dumps(network_list))
            detail = await timed(
                "network_detail_profile_b",
                "network",
                "detail",
                matching_request["request_id"],
                "--include-body",
                "--max-body-bytes",
                "1024",
                *self.target_args(targets[1], tokens[1]),
            )
            self.assertTrue(detail.get("ok"), detail)
            self.assertLessEqual(detail.get("returned_size", 0), 1024)
            self.assertNotIn("python-network-secret", json.dumps(detail))
            network_stop = await timed(
                "network_stop_profile_b",
                "network",
                "stop",
                *self.target_args(targets[1], tokens[1]),
            )
            self.assertTrue(network_stop.get("ok"), network_stop)
            network_started = False
            samples = metrics["working_set_bytes"]
            assert isinstance(samples, list)
            values = [item["bytes"] for item in samples if item.get("bytes")]
            metrics["peak_working_set_bytes"] = max(values, default=None)
            self._log("stage5_6_python_baseline", metrics)
        finally:
            if network_started:
                with contextlib.suppress(Exception):
                    await self.cli(
                        "network",
                        "stop",
                        *self.target_args(targets[1], tokens[1]),
                    )
            await asyncio.gather(
                self.cli(
                    "lease",
                    "release",
                    "--session",
                    targets[0]["session"],
                    "--lease-token",
                    tokens[0],
                ),
                self.cli(
                    "lease",
                    "release",
                    "--session",
                    targets[1]["session"],
                    "--lease-token",
                    tokens[1],
                ),
            )


if __name__ == "__main__":
    unittest.main()
