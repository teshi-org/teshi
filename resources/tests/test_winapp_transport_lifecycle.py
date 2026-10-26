"""Real CLI/daemon transport regressions with isolated services and no native input."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
import venv

REPO = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("TESHI_TEST_CLI", REPO / "target/debug" /
                          ("teshi.exe" if os.name == "nt" else "teshi"))).resolve()
SERVICE = REPO / "resources/winapp_service.py"


@unittest.skipUnless(CLI.is_file(), "build teshi-cli or set TESHI_TEST_CLI")
class TransportLifecycleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="teshi-transport-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def health(self, environment=None):
        result = subprocess.run([str(CLI), "winapp", "health"], cwd=self.root,
                                env=environment, capture_output=True, text=True,
                                encoding="utf-8", timeout=25)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(json.loads(result.stdout)["ok"])

    def wait_for_file(self, path, process):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if path.is_file():
                return
            self.assertIsNone(process.poll(), "owned service exited before readiness")
            time.sleep(0.02)
        self.fail(f"owned service did not create {path.name}")

    def test_one_shot_cli_connections_close_without_server_errors(self):
        bootstrap = (
            f"import sys,asyncio;sys.path.insert(0,{str(SERVICE.parent)!r});"
            "import winapp_service as s;"
            f"asyncio.run(s.run_server('127.0.0.1',0,s.Path({str(self.root)!r}),"
            "'isolated-transport-test'))"
        )
        child = subprocess.Popen([sys.executable, "-c", bootstrap],
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            self.wait_for_file(self.root / ".teshi/cdp-endpoint.json", child)
            # Leave stderr unread, matching the launcher that originally deadlocked.
            for _ in range(20):
                self.health()
        finally:
            child.terminate()
            _, errors = child.communicate(timeout=5)
        self.assertNotIn(b"connection handler failed", errors)

    @unittest.skipUnless(os.name == "nt", "managed WinApp runtime is Windows-only")
    def test_daemon_drains_stderr_before_service_readiness(self):
        self.check_noisy_managed_service(startup_noise=True)

    @unittest.skipUnless(os.name == "nt", "managed WinApp runtime is Windows-only")
    def test_daemon_drains_stderr_after_service_readiness(self):
        self.check_noisy_managed_service(startup_noise=False)

    def check_noisy_managed_service(self, startup_noise):
        runtime = self.root / "runtime"
        venv.EnvBuilder(with_pip=False).create(runtime / "python")
        wrapper = runtime / "service.py"
        wrapper.write_text(
            "import sys\n"
            f"sys.path[:0] = {[str(SERVICE.parent), *sys.path]!r}\n"
            "import winapp_service as service\n"
            "def noise():\n"
            " sys.stderr.write('diagnostic flood ' * 65536)\n"
            " sys.stderr.flush()\n"
            + ("noise()\n" if startup_noise else
               "original = service.WinAppSupervisor.execute\n"
               "async def execute(self, payload, **kwargs):\n"
               " noise()\n"
               " return await original(self, payload, **kwargs)\n"
               "service.WinAppSupervisor.execute = execute\n")
            + "service.main()\n", encoding="utf-8")
        (runtime / "runtime.json").write_text(json.dumps({
            "runtime": "winapp", "runtime_version": 5, "platform": "windows-x86_64",
            "python_exe": "python/Scripts/python.exe", "service_script": wrapper.name,
        }), encoding="utf-8")
        environment = dict(os.environ, TESHI_WINAPP_RUNTIME_DIR=str(runtime),
                           TESHI_APP_DATA_DIR=str(self.root / "app-data"))
        with (self.root / "daemon.log").open("wb") as log:
            daemon = subprocess.Popen([
                str(CLI), "--daemon-internal", "--project-root", str(self.root),
                "--port", "0", "--host", "127.0.0.1",
            ], cwd=self.root, env=environment, stdout=log, stderr=log)
            try:
                self.wait_for_file(self.root / ".teshi/daemon.json", daemon)
                for _ in range(5):
                    self.health(environment)
            finally:
                try:
                    subprocess.run([str(CLI), "daemon", "stop"], cwd=self.root,
                                   env=environment, capture_output=True, timeout=10)
                finally:
                    try:
                        daemon.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        daemon.terminate()
                        daemon.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
