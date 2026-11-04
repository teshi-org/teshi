"""Real subprocess regressions for deadline, framing, diagnostics and ownership."""

from contextlib import nullcontext, redirect_stdout
import ctypes
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from ci_process import HostClient


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ROOT = Path(__file__).resolve().parents[2]
supervisor = load("ci_command", ROOT / "scripts/run-ci-command.py")
bindings = load("directory_feature_steps", ROOT / "crates/teshi-tui-runner/tests/fixtures/directory_feature_steps.py")


class ProcessProbe:
    def __init__(self, pid):
        self.pid = pid
        if os.name == "nt":
            self.api = ctypes.WinDLL("kernel32", use_last_error=True)
            self.api.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
            self.api.OpenProcess.restype = ctypes.c_void_p
            self.api.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
            self.api.CloseHandle.argtypes = [ctypes.c_void_p]
            self.handle = self.api.OpenProcess(0x00100000, False, pid)
            if not self.handle:
                raise ctypes.WinError(ctypes.get_last_error())

    def alive(self):
        if os.name == "nt":
            return self.api.WaitForSingleObject(self.handle, 0) == 258
        try:
            os.kill(self.pid, 0)
            # A terminated orphan can remain a zombie until init reaps it.
            stat = Path(f"/proc/{self.pid}/stat")
            return not stat.exists() or stat.read_text().split(") ", 1)[1][0] != "Z"
        except ProcessLookupError:
            return False

    def close(self):
        if os.name == "nt":
            self.api.CloseHandle(self.handle)


class HostDeadlineTests(unittest.TestCase):
    def host(self, script, root):
        client = HostClient([sys.executable, "-u", "-c", script], root)
        self.addCleanup(client.close)
        return client

    def test_success_and_valid_error_preserve_request_framing(self):
        with tempfile.TemporaryDirectory() as root:
            client = self.host('import sys,json\nfor line in sys.stdin:\n r=json.loads(line); print(json.dumps({"ok":r["op"]!="reject","value":"viewport","error":"expected rejection"}),flush=True)', root)
            self.assertEqual(client.request(dict(op="screen"), time.monotonic() + 2), "viewport")
            with self.assertRaisesRegex(AssertionError, "expected rejection"):
                client.request(dict(op="reject"), time.monotonic() + 2)
            self.assertFalse(client.poisoned)
            self.assertEqual(client.request(dict(op="screen"), time.monotonic() + 2), "viewport")
            client.close()

    def test_silent_host_timeout_saves_context_and_never_reuses_late_reply(self):
        with tempfile.TemporaryDirectory() as root:
            client = self.host('import sys,json,time\nfor line in sys.stdin:\n r=json.loads(line)\n if r["op"]=="stall": time.sleep(60)\n print(json.dumps({"ok":True,"value":"last viewport"}),flush=True)', root)
            client.request(dict(op="screen"), time.monotonic() + 2)
            started = time.monotonic()
            with self.assertRaises(TimeoutError):
                client.request(dict(op="stall"), started + .15, dict(case_id="owned", step_line=42))
            self.assertLess(time.monotonic() - started, 2)
            with self.assertRaises(RuntimeError):
                client.request(dict(op="screen"), time.monotonic() + 2)
            diagnostic = json.loads((Path(root) / "timeout-diagnostic.json").read_text())
            self.assertEqual(diagnostic["current"]["context"]["step_line"], 42)
            client.diagnose(RuntimeError("secondary cleanup failure"))
            self.assertEqual(json.loads((Path(root) / "timeout-diagnostic.json").read_text())["error_type"], "TimeoutError")
            self.assertEqual(len((Path(root) / "failures.ndjson").read_text().splitlines()), 2)
            self.assertEqual((Path(root) / "last-successful-screen.txt").read_text(), "last viewport")
            client.close()
            self.assertIsNotNone(client.process.poll())

    def test_blocked_pipe_write_is_bounded_too(self):
        with tempfile.TemporaryDirectory() as root:
            client = self.host("import time; time.sleep(60)", root)
            started = time.monotonic()
            with self.assertRaises(TimeoutError):
                client.request(dict(op="write", text="x" * 2_000_000), started + .15)
            self.assertLess(time.monotonic() - started, 2)
            client.close()
            self.assertFalse(client.worker.is_alive())

    def test_eof_and_malformed_json_fail_without_waiting_for_deadline(self):
        for body in ("pass", 'print("malformed", flush=True)'):
            with self.subTest(body=body), tempfile.TemporaryDirectory() as root:
                client = self.host("import sys; sys.stdin.readline(); " + body, root)
                with self.assertRaises((EOFError, json.JSONDecodeError)):
                    client.request(dict(op="screen"), time.monotonic() + 3)
                self.assertTrue(client.poisoned)
                client.close()

    def test_viewport_wait_shares_its_deadline_with_host_read(self):
        with tempfile.TemporaryDirectory() as root:
            client = self.host("import time; time.sleep(60)", root)
            world = bindings.World.__new__(bindings.World)
            world.client, world.context, world.request_timeout = client, {}, 30
            world.deadline = time.monotonic() + 30
            world.spans, world.timings = [], dict(wait=0)
            started = time.monotonic()
            with self.assertRaises(TimeoutError):
                world.wait(lambda screen: False, timeout=.15)
            self.assertLess(time.monotonic() - started, 2)
            self.assertGreater(world.timings["wait"], .1)
            client.close()

    def test_cleanup_kills_owned_descendant_after_root_exit_and_preserves_unrelated_process(self):
        unrelated = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
        try:
            with tempfile.TemporaryDirectory() as root:
                client = self.host('import subprocess,sys,json\nsys.stdin.readline()\np=subprocess.Popen([sys.executable,"-c","import time; time.sleep(60)"])\nprint(json.dumps({"ok":True,"value":p.pid}),flush=True)\nsys.stdin.readline()', root)
                pid = client.request(dict(op="child"), time.monotonic() + 3)
                probe = ProcessProbe(pid)
                try:
                    client.process.stdin.write("exit\n")
                    client.process.stdin.flush()
                    client.process.wait(timeout=3)
                    self.assertTrue(probe.alive())
                    client.close()
                    deadline = time.monotonic() + 3
                    while probe.alive() and time.monotonic() < deadline:
                        time.sleep(.02)
                    self.assertFalse(probe.alive(), "owned descendant survived cleanup")
                    self.assertIsNone(unrelated.poll())
                finally:
                    probe.close()
        finally:
            unrelated.terminate()
            unrelated.wait(timeout=3)

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux detached descendant cleanup")
    def test_timeout_cleanup_collects_a_descendant_in_a_separate_session(self):
        with tempfile.TemporaryDirectory() as root:
            client = self.host('import subprocess,sys,json,time\nsys.stdin.readline()\np=subprocess.Popen([sys.executable,"-c","import time; time.sleep(60)"],start_new_session=True)\nprint(json.dumps({"ok":True,"value":p.pid}),flush=True)\ntime.sleep(60)', root)
            pid = client.request(dict(op="child"), time.monotonic() + 3)
            probe = ProcessProbe(pid)
            try:
                client.close()
                deadline = time.monotonic() + 3
                while probe.alive() and time.monotonic() < deadline:
                    time.sleep(.02)
                self.assertFalse(probe.alive(), "detached descendant escaped cleanup")
            finally:
                probe.close()


class CommandSupervisorTests(unittest.TestCase):
    def test_spawn_failure_retains_failure_evidence(self):
        with tempfile.TemporaryDirectory() as root:
            code = supervisor.run([str(Path(root) / "missing-executable")], Path(root), "missing", 3)
            self.assertEqual(code, 125)
            self.assertIn("startup_error", json.loads((Path(root) / "timings.json").read_text()))
            self.assertTrue((Path(root) / "junit.xml").exists())

    def test_actual_serial_output_retains_observed_test_durations(self):
        with tempfile.TemporaryDirectory() as root, redirect_stdout(io.StringIO()):
            script = "import time; print('test real_case ... ',end='',flush=True); time.sleep(.15); print('ok',flush=True)"
            code = supervisor.run([sys.executable, "-u", "-c", script], Path(root), "serial", 3, True)
            self.assertEqual(code, 0)
            record = json.loads((Path(root) / "timings.json").read_text())
            self.assertEqual(record["tests"][0]["name"], "real_case")
            self.assertGreater(record["tests"][0]["elapsed_seconds"], .1)

    def test_timeout_and_child_exit_failure_are_reported_in_json_and_junit(self):
        for script, timeout, expected in (("import time; time.sleep(60)", .15, 124),
                                          ("import sys; print('failed'); sys.exit(7)", 3, 7),
                                          ("print('passed')", 3, 0)):
            with self.subTest(expected=expected), tempfile.TemporaryDirectory() as root:
                with redirect_stdout(io.StringIO()):
                    code = supervisor.run([sys.executable, "-u", "-c", script], Path(root), "probe", timeout)
                self.assertEqual(code, expected)
                record = json.loads((Path(root) / "timings.json").read_text())
                self.assertEqual(record["timed_out"], expected == 124)
                self.assertTrue((Path(root) / "junit.xml").exists())

    def test_closed_stdout_does_not_bypass_command_timeout(self):
        with tempfile.TemporaryDirectory() as root, redirect_stdout(io.StringIO()):
            started = time.monotonic()
            code = supervisor.run([sys.executable, "-u", "-c", "import os,time; os.close(1); os.close(2); time.sleep(60)"], Path(root), "closed-output", .2)
            self.assertEqual(code, 124)
            self.assertLess(time.monotonic() - started, 2)

    def test_split_serial_libtest_output_tracks_active_test_and_ignored_status(self):
        times = supervisor.TestTimes()
        times.feed("test alpha ... ", 10)
        self.assertEqual(times.active[0], "alpha")
        times.feed("ok\ntest beta ... ", 12)
        times.feed("FAILED\ntest gamma ... ignored, reason\n", 15)
        self.assertEqual([item["elapsed_seconds"] for item in times.tests], [2, 3, 0])
        self.assertEqual(times.tests[2]["result"], "ignored, reason")
        self.assertIsNone(times.active)


class DirectoryAccountingTests(unittest.TestCase):
    def test_cleanup_failure_is_a_failed_example_in_events_and_junit(self):
        class FakeWorld:
            def __init__(self, request, evidence):
                self.client = Mock()
                self.root = Path(evidence)
                self.case_deadline = time.monotonic() + 60
                self.step_timeout = 30
                self.timings = {}

            def measure(self, phase):
                return nullcontext()

            def step(self, text):
                pass

            def capture(self, *args, **kwargs):
                return dict(capture_status="available")

            def close(self):
                raise RuntimeError("owned cleanup failed")

        request = dict(meta={}, cases=[dict(id="case", scenario="example", feature_path="fixture.feature",
            steps=[dict(line=4, text="do <value>")], examples=[dict(headers=["value"], rows=[["one"], ["two"]])])])
        with tempfile.TemporaryDirectory() as root, patch.dict(os.environ, TESHI_FEATURE_ARTIFACTS=root), \
                patch.object(bindings.sys, "stdin", io.StringIO(json.dumps(request))), \
                patch.object(bindings, "World", FakeWorld), redirect_stdout(io.StringIO()) as output:
            self.assertTrue(bindings.main())
            records = [json.loads(line) for line in output.getvalue().splitlines()]
            self.assertEqual(records[-1]["examples_failed"], 2)
            self.assertEqual(records[-1]["examples_passed"], 0)
            suite = supervisor.ET.parse(Path(root) / "junit.xml").getroot()
            self.assertEqual(suite.get("tests"), "2")
            self.assertEqual(suite.get("failures"), "2")

    def test_nested_wait_time_is_not_double_counted_as_execution(self):
        world = bindings.World.__new__(bindings.World)
        world.spans, world.timings = [], dict(execution=0, wait=0)
        with patch.object(bindings.time, "monotonic", side_effect=[0, 1, 4, 6]):
            with world.measure("execution"):
                with world.measure("wait"):
                    pass
        self.assertEqual(world.timings, dict(execution=3, wait=3))


if __name__ == "__main__":
    unittest.main()
