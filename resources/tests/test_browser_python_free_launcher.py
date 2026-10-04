"""Explicit Windows acceptance of the ordinary per-user Rust Chrome launcher.

This requires a free discovery port and rotates stale per-user runtime records.
Durable pairing and unrelated processes are preserved. Run the named test only.
"""

from __future__ import annotations

import asyncio
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import unittest

from resources.tests import test_browser_two_profile_rust_transport as transport

TESHI_CLI = transport.TESHI_CLI


class OwnedWindowsBroker:
    """Hold the exact launched process handle so cleanup cannot hit a reused PID."""

    stdout = None
    stderr = None

    def __init__(self, pid: int, expected_executable: Path):
        self.pid = pid
        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        self.api.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        self.api.OpenProcess.restype = wintypes.HANDLE
        self.api.CloseHandle.argtypes = [wintypes.HANDLE]
        self.api.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
        self.api.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
        self.api.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
        self.api.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        self.handle = self.api.OpenProcess(0x00100000 | 0x1000 | 0x1, False, pid)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        buffer = ctypes.create_unicode_buffer(32_768)
        length = wintypes.DWORD(len(buffer))
        if not self.api.QueryFullProcessImageNameW(self.handle, 0, buffer, ctypes.byref(length)):
            self.close()
            raise ctypes.WinError(ctypes.get_last_error())
        if Path(buffer.value).resolve() != expected_executable.resolve():
            self.close()
            raise AssertionError("launcher PID does not belong to the acceptance CLI copy")

    @property
    def returncode(self):
        return self.poll()

    def poll(self):
        code = wintypes.DWORD()
        if not self.api.GetExitCodeProcess(self.handle, ctypes.byref(code)):
            raise ctypes.WinError(ctypes.get_last_error())
        return None if code.value == 259 else code.value

    def terminate(self):
        if not self.api.TerminateProcess(self.handle, 0):
            raise ctypes.WinError(ctypes.get_last_error())

    kill = terminate

    def wait(self, timeout):
        result = self.api.WaitForSingleObject(self.handle, int(timeout * 1000))
        if result == 258:
            raise subprocess.TimeoutExpired("owned Rust broker", timeout)
        if result != 0:
            raise ctypes.WinError(ctypes.get_last_error())
        return self.poll()

    def close(self):
        if self.handle:
            self.api.CloseHandle(self.handle)
            self.handle = None


@unittest.skipUnless(
    os.name == "nt" and os.environ.get("TESHI_RUN_NORMAL_CHROME_LAUNCHER_ACCEPTANCE") == "1",
    "explicit Windows acceptance requires TESHI_RUN_NORMAL_CHROME_LAUNCHER_ACCEPTANCE=1",
)
class PythonFreeLauncherTests(transport.BrowserTwoProfileRustTransportTests):
    @staticmethod
    def _unused_port() -> int:
        return 17373

    async def asyncSetUp(self):
        self._assert_discovery_free()
        self.pairing_path = Path(os.environ["LOCALAPPDATA"]) / "teshi/browser-broker/pairing.json"
        self.pairing_before = self._pairing_digest()
        await super().asyncSetUp()

    def _pairing_digest(self):
        return hashlib.sha256(self.pairing_path.read_bytes()).hexdigest() if self.pairing_path.exists() else None

    def _assert_discovery_free(self):
        with socket.socket() as probe:
            probe.settimeout(1)
            self.assertNotEqual(probe.connect_ex(("127.0.0.1", 17373)), 0,
                "existing discovery listener must be left untouched")

    def _invoke_launcher(self):
        """Wait for the CLI itself, without waiting for detached-child pipe EOF."""
        args = [str(self.launcher_cli), "browser", "sessions"]
        self.launcher_runs = getattr(self, "launcher_runs", 0) + 1
        output_root = self.artifact_dir or self.temp_root
        stdout_path = output_root / f"launcher-{self.launcher_runs}-stdout.log"
        stderr_path = output_root / f"launcher-{self.launcher_runs}-stderr.log"
        timed_out = False
        with stdout_path.open("w", encoding="utf-8") as stdout, stderr_path.open("w", encoding="utf-8") as stderr:
            child = subprocess.Popen(args, cwd=self.temp_root, env=self.broker_env,
                stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr, text=True)
            try:
                child.wait(timeout=30)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=5)
                timed_out = True
        # Sessions output and launch diagnostics contain no private credential.
        return subprocess.CompletedProcess(args, 124 if timed_out else child.returncode,
            stdout_path.read_text(encoding="utf-8"),
            "launcher timed out" if timed_out else stderr_path.read_text(encoding="utf-8"))

    async def _start_broker(self):
        self._assert_discovery_free()
        self.launcher_cli = self.temp_root / "teshi.exe"
        if not self.launcher_cli.exists():
            shutil.copyfile(TESHI_CLI, self.launcher_cli)
        self.python_free = True
        self.broker_env = os.environ.copy()
        self.broker_env["PATH"] = str(Path(os.environ["SYSTEMROOT"]) / "System32")
        self.broker_env["TESHI_BROWSER_BROKER_IMPLEMENTATION"] = "rust"
        self.broker_env["TESHI_BROWSER_BROKER_EXTENSION_ORIGINS"] = self.extension_origin
        for key in ("PYTHONPATH", "PYTHONHOME", "VIRTUAL_ENV", "TESHI_BROWSER_BROKER_USE_EXISTING_ENDPOINT"):
            self.broker_env.pop(key, None)
        for name in ("python", "python3", "py", "pip", "pip3", "uv", "playwright"):
            self.assertIsNone(shutil.which(name, path=self.broker_env["PATH"]), name)
        self.assertFalse((self.temp_root / ".venv").exists())
        launcher = await asyncio.to_thread(self._invoke_launcher)
        pointer = self.temp_root / ".teshi/cdp-endpoint.json"
        if pointer.exists():
            self.ready_endpoint = json.loads(pointer.read_text(encoding="utf-8-sig"))
            self.broker = OwnedWindowsBroker(self.ready_endpoint["broker_pid"], self.launcher_cli)
        self.assertEqual(launcher.returncode, 0, self._redact(launcher.stderr))
        self.assertIsNotNone(self.broker)
        self.assertNotIn("token=", self.ready_endpoint["ws_url"])
        self.assertIn("transport.v1", self.ready_endpoint["broker_features"])
        self.assertIn("p0.control", self.ready_endpoint["broker_features"])
        trusted = await asyncio.to_thread(self._http_discovery, self.extension_origin, "POST")
        self.assertEqual(trusted["status"], 200)
        self.assertEqual(trusted["payload"]["broker_pid"], self.broker.pid)
        self.assertEqual(trusted["payload"]["broker_start_id"], self.ready_endpoint["broker_start_id"])
        self.control_url = trusted["payload"]["ws_url"]
        self.old_token = self._query_token(self.control_url)
        self._log("normal_python_free_launcher_ready", {
            "pid": self.broker.pid, "broker_start_id": self.ready_endpoint["broker_start_id"],
            "python_on_runtime_path": False, "project_venv_present": False,
            "public_pointer_has_credentials": False, "development_rust_selector": True,
        })

    async def _stop_broker(self):
        owned = self.broker
        try:
            await super()._stop_broker()
        finally:
            if owned is not None:
                owned.close()

    async def _cleanup(self):
        try:
            await super()._cleanup()
        finally:
            self.assertEqual(self._pairing_digest(), self.pairing_before, "durable pairing was changed")
            self._log("durable_pairing_preserved", {"unchanged": True})

    async def test_normal_launcher_and_agent_acceptance(self):
        await self.test_two_projects_fill_evidence_and_reconnect()
        # Legacy Agent tools select the only ready Profile. Close only the second
        # test-owned context, preserving Profile A and the shared broker generation.
        second = self.contexts.pop("profile-b")
        await asyncio.wait_for(second.close(), 10)
        self.workers.pop("profile-b", None)
        remaining = await self._wait_sessions(1, expected_count=1)
        profile_id = self._target(self._session_for_path(remaining, "/project-a"))["extension_instance_id"]
        test_executable = Path(os.environ["TESHI_AGENT_ACCEPTANCE_TEST_EXECUTABLE"]).resolve()
        self.assertTrue(test_executable.is_file())
        test_env = os.environ.copy()
        test_env.update({
            "TESHI_BROWSER_BROKER_IMPLEMENTATION": "rust",
            "TESHI_BROWSER_BROKER_USE_EXISTING_ENDPOINT": "1",
            "TESHI_ACCEPTANCE_PROJECT": str(self.temp_root),
            "TESHI_RUN_LIVE_CHROME_AGENT_E2E": "1",
            "TESHI_EXPECT_BROKER_START_ID": self.ready_endpoint["broker_start_id"],
            "TESHI_EXPECT_EXTENSION_INSTANCE_ID": profile_id,
            "TESHI_EXPECT_BROWSER_URL_PATTERN": re.escape(f"127.0.0.1:{self.http.server_address[1]}/project-a"),
        })
        api_key = test_env.get("DEEPSEEK_API_KEY", "")
        self.assertTrue(api_key, "live Agent acceptance requires configured DEEPSEEK_API_KEY")
        # The production dispatcher checks the shared configuration before it
        # submits to the already-attached model. Keep this credential in memory.
        test_env["TESHI_LLM_API_KEY"] = api_key
        test_env["TESHI_LLM_BASE_URL"] = "https://api.deepseek.com"
        test_env["TESHI_LLM_MODEL"] = "deepseek-flash"
        for test_name, flags in (
            ("app::tests::native_model_browser_tool_round_trip_uses_the_shared_broker", ["--ignored"]),
            ("app::tests::live_chrome_agent_broker_identity", []),
        ):
            self._set_stage(test_name)
            result = await asyncio.to_thread(subprocess.run,
                [str(test_executable), test_name, "--exact", "--nocapture", *flags],
                cwd=self.temp_root, env=test_env, stdin=subprocess.DEVNULL,
                capture_output=True, text=True, timeout=300)
            output = self._redact(result.stdout + result.stderr).replace(api_key, "[REDACTED]")
            if self.artifact_dir is not None:
                (self.artifact_dir / f"{test_name.rsplit('::', 1)[1]}.log").write_text(output, encoding="utf-8")
            self.assertEqual(result.returncode, 0, output)
            self.assertIn("1 passed", output)
            self._log("agent_acceptance_passed", {"test": test_name,
                "broker_start_id": self.ready_endpoint["broker_start_id"],
                "extension_instance_id": profile_id})
