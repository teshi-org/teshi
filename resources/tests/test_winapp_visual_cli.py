"""Black-box CLI/replay JSON contracts against an isolated fake WebSocket sidecar."""
import json
import logging
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest

from websockets.sync.server import serve

REPO = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("TESHI_TEST_CLI", REPO / "target/debug" / ("teshi.exe" if os.name == "nt" else "teshi")))


@unittest.skipUnless(CLI.is_file(), "build teshi-cli or set TESHI_TEST_CLI for CLI contracts")
class VisualCliTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / ".teshi").mkdir()
        self.commands = []
        self.disconnect = False
        self.capture_failure = False
        self.fail_action_number = None
        self.action_count = 0
        self.health_response = None
        self.stall_commands = set()
        self.release_stall = threading.Event()
        self.capture_count = 0
        self.fail_capture_number = None
        self.response = {"ok": True, "bounds": {"x": 20, "y": 30, "width": 2, "height": 3}, "dpi": 144}
        logger = logging.getLogger("winapp-test-sidecar")
        logger.setLevel(logging.CRITICAL)

        def handler(socket):
            command = json.loads(socket.recv())
            self.commands.append(command)
            if command["cmd"] in self.stall_commands:
                self.release_stall.wait(5)
                return
            if self.disconnect:
                return
            if command["cmd"] == "health":
                response = self.health_response or {"ok": True, "protocol": "teshi-winapp", "version": 1, "project": str(self.root)}
            elif command["cmd"] == "get_target":
                response = {"ok": True, "target": {"attached": True}}
            elif command["cmd"] == "screenshot":
                self.capture_count += 1
                response = ({"ok": False, "error": "injected capture failure"} if self.capture_failure or self.capture_count == self.fail_capture_number
                            else {"ok": True, "screenshot": "AA=="})
            elif (command.get("action") == "pointer_click" or command["cmd"] == "shell_tray_click") and not (
                command.get("mode") == "foreground" and command.get("allow_interactive_input") is True
            ):
                response = {"ok": False, "code": "requires_interactive_input", "error": "Explicit input permission required"}
            elif command.get("action") in (
                "click", "pointer_click", "fill", "assert_visible", "assert_not_exists", "assert_text"
            ):
                response = {"ok": True}
            else:
                response = self.response
            if command["cmd"] == "execute_locator":
                self.action_count += 1
                if self.action_count == self.fail_action_number:
                    response = {"ok": False, "error": "injected action failure", "code": "assertion_failed"}
            socket.send(json.dumps({"type": "response", **response}))

        self.server = serve(handler, "127.0.0.1", 0, logger=logger)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.stop_server)
        port = self.server.socket.getsockname()[1]
        (self.root / ".teshi/cdp-endpoint.json").write_text(json.dumps({
            "mode": "winapp", "ws_url": f"ws://127.0.0.1:{port}", "page_url": "winapp://test", "updated_at_ms": 0,
        }), encoding="utf-8")

    def stop_server(self):
        self.release_stall.set()
        self.server.shutdown()
        self.thread.join(timeout=5)

    def cli(self, *args):
        return subprocess.run([str(CLI), *args], cwd=self.root, capture_output=True, text=True, encoding="utf-8", timeout=30)

    def test_screenshot_json_success_and_failure(self):
        args = ("winapp", "screenshot", "--selector", "uia:name=Close", "--out", "out.png")
        success = self.cli(*args)
        self.assertEqual(success.returncode, 0, success.stderr)
        self.assertTrue(json.loads(success.stdout)["ok"])
        self.assertEqual(self.commands[-1]["cmd"], "element_screenshot")
        self.response = {"ok": False, "error": "ambiguous visible selector"}
        failure = self.cli(*args)
        self.assertNotEqual(failure.returncode, 0)
        self.assertFalse(json.loads(failure.stdout)["ok"])

    def test_assertion_json_and_nonzero_exit(self):
        self.response.update(ok=False, error="screenshot pixels differ", changed_pixels=2,
                             baseline_dimensions={"width": 2, "height": 3},
                             actual_dimensions={"width": 2, "height": 3}, diff_out="diff.png")
        result = self.cli("winapp", "assert-screenshot", "--selector", "uia:name=Close", "--baseline", "base.png",
                          "--diff-out", "diff.png", "--pixel-tolerance", "8")
        self.assertNotEqual(result.returncode, 0)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["changed_pixels"], 2)
        self.assertFalse(payload["ok"])
        self.assertEqual(self.commands[-1]["pixel_tolerance"], 8)
        executed = self.cli("winapp", "execute", "--selector", "uia:name=Close", "--action", "assert_screenshot", "--value-arg", "base.png")
        self.assertNotEqual(executed.returncode, 0)
        self.assertFalse(json.loads(executed.stdout)["ok"])
        self.response["ok"] = True
        self.assertEqual(self.cli("winapp", "assert-screenshot", "--selector", "uia:name=Close", "--baseline", "base.png",
                                 "--diff-out", "diff.png").returncode, 0)

    def test_transport_failure_still_emits_json(self):
        self.disconnect = True
        result = self.cli("winapp", "screenshot", "--selector", "uia:name=Close", "--out", "out.png")
        self.assertNotEqual(result.returncode, 0)
        payload = json.loads(result.stdout)
        self.assertFalse(payload["ok"])
        self.assertIn("error", payload)

    def test_pointer_click_binding_and_replay_preserve_action(self):
        (self.root / "pointer.feature").write_text(
            "Feature: Pointer action\n"
            "  Scenario: Click the caption close button\n"
            "    Given the caption is visible\n"
            "    When the close button is clicked\n",
            encoding="utf-8",
        )
        for line, action, rationale in [
            (3, "assert_visible", "Read-only assertion"),
            (4, "pointer_click", "Requires real pointer hover and pressed state"),
        ]:
            selected = self.cli(
                "steps", "select", "--feature", "pointer.feature", "--line", str(line)
            )
            self.assertEqual(selected.returncode, 0, selected.stderr)
            proposed = self.cli(
                "steps", "propose", "--line", str(line), "--strategy", "uia",
                "--value", "uia:control_type=ButtonControl;name=Close",
                "--action", action, "--confidence", "1", "--rationale", rationale,
            )
            self.assertEqual(proposed.returncode, 0, proposed.stderr)
            confirmed = self.cli("steps", "confirm", "--rank", "1")
            self.assertEqual(confirmed.returncode, 0, confirmed.stderr)

        binding_files = list(self.root.rglob("*.bindings.json"))
        self.assertTrue(binding_files)
        binding = json.loads(binding_files[0].read_text(encoding="utf-8"))
        self.assertEqual(binding["steps"][0]["primary"]["action"], "assert_visible")
        self.assertEqual(binding["steps"][1]["primary"]["action"], "pointer_click")

        replay = self.cli(
            "winapp", "replay", "--feature", "pointer.feature", "--non-interactive"
        )
        self.assertNotEqual(replay.returncode, 0)
        self.assertIn("requires_interactive_input", replay.stdout)
        self.assertIn('"status": "blocked"', replay.stdout)
        self.assertFalse(any(c.get("action") == "pointer_click" for c in self.commands))
        replay = self.cli("winapp", "replay", "--feature", "pointer.feature", "--yes",
                          "--mode", "foreground", "--allow-interactive-input")
        self.assertEqual(replay.returncode, 0, replay.stderr)
        pointer_commands = [
            command for command in self.commands
            if command.get("action") == "pointer_click"
        ]
        self.assertTrue(pointer_commands)
        self.assertEqual(pointer_commands[-1]["action"], "pointer_click")
        self.assertEqual(pointer_commands[-1]["mode"], "foreground")
        self.assertIs(pointer_commands[-1]["allow_interactive_input"], True)
        assertion_commands = [
            command for command in self.commands
            if command.get("action") == "assert_visible"
        ]
        self.assertTrue(assertion_commands)
        self.assertEqual(assertion_commands[-1]["mode"], "foreground")

    def test_input_permission_execute_and_tray_json_contract(self):
        for arguments in (
            ("execute", "--selector", "uia:name=Close", "--action", "pointer_click"),
            ("shell", "tray", "click", "--name", "Fixture"),
        ):
            self.commands.clear()
            denied = self.cli("winapp", *arguments)
            self.assertNotEqual(denied.returncode, 0)
            self.assertEqual(json.loads(denied.stdout)["code"], "requires_interactive_input")
            self.assertFalse(self.commands or [], "CLI must reject before contacting a sidecar")
            self.commands.clear()
            granted = self.cli("winapp", *arguments, "--mode", "foreground", "--allow-interactive-input")
            self.assertEqual(granted.returncode, 0, granted.stderr)
            self.assertIs(self.commands[-1]["allow_interactive_input"], True)

    def test_replay_visual_failure_and_legacy_actions(self):
        (self.root / "visual.feature").write_text(
            "Feature: Caption appearance\n  Scenario: Compare caption\n    Given the caption is visible\n\n"
            "    When the caption is inspected\n\n    Then the caption matches its baseline\n\n"
            "    Then the missing banner is absent\n", encoding="utf-8")
        for line, action in [(3, "assert_visible"), (5, "click"), (7, "assert_screenshot"), (9, "assert_not_exists")]:
            selected = self.cli("steps", "select", "--feature", "visual.feature", "--line", str(line))
            self.assertEqual(selected.returncode, 0, selected.stderr)
            proposed = self.cli("steps", "propose", "--line", str(line), "--strategy", "uia", "--value", "uia:name=Close",
                                "--action", action, "--value-arg", "base.png", "--confidence", "1", "--rationale", "Isolated protocol fixture")
            self.assertEqual(proposed.returncode, 0, proposed.stderr)
            confirmed = self.cli("steps", "confirm", "--rank", "1")
            self.assertEqual(confirmed.returncode, 0, confirmed.stderr)
        self.response.update(ok=False, error="screenshot pixels differ", changed_pixels=3)
        failure = self.cli("winapp", "replay", "--feature", "visual.feature", "--non-interactive")
        self.assertNotEqual(failure.returncode, 0)
        self.assertIn('"changed_pixels": 3', failure.stdout)
        self.assertIn("line 7", failure.stderr)
        visual = next(c for c in self.commands if c.get("action") == "assert_screenshot")
        self.assertEqual(visual["value"], "base.png")
        self.assertEqual(visual["pixel_tolerance"], 8)
        self.assertTrue(visual["diff_out"].endswith("visual.feature-L7-diff.png"), visual)
        self.response["ok"] = True
        success = self.cli("winapp", "replay", "--feature", "visual.feature", "--non-interactive")
        self.assertEqual(success.returncode, 0, success.stderr)
        for action in ("click", "fill", "assert_visible", "assert_not_exists", "assert_text"):
            self.assertEqual(self.cli("winapp", "execute", "--selector", "uia:name=Close", "--action", action,
                                     "--value-arg", "text").returncode, 0)
            self.assertNotIn("pixel_tolerance", self.commands[-1])
            self.assertNotIn("diff_out", self.commands[-1])

    def fixture(self, source, lines):
        (self.root / "coverage.feature").write_text(source, encoding="utf-8")
        checked = self.cli("check", "--feature", "coverage.feature", "--json")
        self.assertEqual(checked.returncode, 0, checked.stderr + checked.stdout)
        for line in lines:
            for args in [
                ("steps", "select", "--feature", "coverage.feature", "--line", str(line)),
                ("steps", "propose", "--line", str(line), "--strategy", "uia", "--value", "uia:name=<name>",
                 "--action", "assert_visible", "--confidence", "1", "--rationale", "Isolated coverage fixture"),
                ("steps", "confirm", "--rank", "1")]:
                result = self.cli(*args)
                self.assertEqual(result.returncode, 0, result.stderr)

    def replay_fixture(self, *args):
        return self.cli("winapp", "replay", "--feature", "coverage.feature", "--yes", *args)

    def latest_run(self):
        runs = list((self.root / ".teshi/logs/replay-screenshots").glob("*/run.json"))
        path = max(runs, key=lambda item: item.stat().st_mtime_ns)
        return json.loads(path.read_text()), json.loads(path.with_name("index.json").read_text())

    def test_missing_steps_are_rejected_before_any_action(self):
        source = "Feature: Coverage\n  Scenario: All steps\n    Given ready\n\n    When inspected\n\n    Then visible\n"
        for missing in (3, 5, 7):
            with self.subTest(missing=missing):
                # Each subcase must have a genuinely missing binding.
                import shutil
                shutil.rmtree(self.root / ".teshi/step-bindings", ignore_errors=True)
                (self.root / "coverage.bindings.json").unlink(missing_ok=True)
                self.fixture(source, [line for line in (3, 5, 7) if line != missing])
                result = self.replay_fixture()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('"status": "unbound"', result.stdout)
                self.assertEqual(self.action_count, 0)

    def test_no_bindings_manual_and_dry_run_do_not_execute(self):
        self.fixture("Feature: Coverage\n  Scenario: Empty\n    Given ready\n", [])
        self.assertNotEqual(self.replay_fixture().returncode, 0)
        self.fixture("Feature: Coverage\n  @manual\n  Scenario: Human\n    Given ready\n", [4])
        manual = self.replay_fixture()
        self.assertNotEqual(manual.returncode, 0)
        self.assertIn('"status": "manual"', manual.stdout)
        self.fixture("Feature: Coverage\n  Scenario: Preview\n    Given ready\n", [3])
        preview = self.replay_fixture("--dry-run")
        self.assertEqual(preview.returncode, 0, preview.stderr)
        self.assertEqual(self.action_count, 0)
        self.assertFalse((self.root / ".teshi/logs/replay-screenshots").exists())

    def test_background_outline_and_partial_scope(self):
        self.fixture("Feature: Coverage\n  Background:\n    Given ready\n\n"
                     "  Scenario Outline: Names\n    Then <name> is visible\n\n"
                     "    Examples:\n      | name |\n      | Alpha |\n      | Beta |\n\n"
                     "  Scenario: Later\n    Then finished\n", [3, 6, 14])
        result = self.replay_fixture("--until-line", "6")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('"status": "partial"', result.stdout)
        actions = [c for c in self.commands if c["cmd"] == "execute_locator"]
        self.assertEqual(len(actions), 4)
        self.assertEqual([actions[1]["selector"], actions[3]["selector"]], ["uia:name=Alpha", "uia:name=Beta"])
        run, index = self.latest_run()
        self.assertEqual(run["status"], "partial")
        self.assertEqual(index["status"], "partial")
        identities = [e["step_id"] for e in run["events"] if e["type"] == "step_result"]
        self.assertEqual(len(set(identities)), 4)

    def test_all_captures_fail_but_terminal_is_passed_and_unique(self):
        self.fixture("Feature: Coverage\n  Scenario: Result\n    Given ready\n", [3])
        self.capture_failure = True
        ids = set()
        for _ in range(2):
            result = self.replay_fixture()
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('"event": "replay_complete"', result.stdout)
            run, index = self.latest_run()
            ids.add(run["id"])
            self.assertEqual(run["status"], "passed")
            self.assertEqual(index["status"], "passed")
            self.assertEqual(run["events"][-1]["capture_failures"], 1)
            kinds = [event["type"] for event in run["events"]]
            self.assertLess(kinds.index("step_result"), kinds.index("step_screenshot"))
        self.assertEqual(len(ids), 2)

    def test_first_and_middle_failure_keep_terminal_and_evidence(self):
        self.fixture("Feature: Coverage\n  Scenario: Result\n    Given ready\n\n    Then visible\n", [3, 5])
        for fail in (1, 2):
            self.action_count = 0
            self.fail_action_number = fail
            result = self.replay_fixture()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('"event": "replay_complete"', result.stdout)
            run, index = self.latest_run()
            self.assertEqual(run["status"], "failed")
            self.assertEqual(index["status"], "failed")
            self.assertEqual(len(index["steps"]), fail)
            results = [e for e in run["events"] if e["type"] == "step_result"]
            self.assertEqual(results[-1]["response"]["code"], "assertion_failed")

    def test_partial_capture_failure_does_not_change_action_outcome(self):
        self.fixture("Feature: Coverage\n  Scenario: Result\n    Given ready\n\n    Then visible\n", [3, 5])
        self.fail_capture_number = 2
        result = self.replay_fixture()
        self.assertEqual(result.returncode, 0, result.stderr)
        run, index = self.latest_run()
        self.assertEqual(run["status"], "passed")
        self.assertEqual(len(index["steps"]), 1)
        self.assertEqual(run["events"][-1]["capture_failures"], 1)

    def test_protocol_and_authentication_failures_are_not_mutation_retries(self):
        for response, code in [({"ok": True}, "protocol_mismatch"),
                               ({"ok": False, "code": "authentication_failed", "error": "Bad credential"}, "authentication_failed")]:
            self.health_response = response
            result = self.cli("winapp", "execute", "--selector", "uia:name=Close", "--action", "click")
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(json.loads(result.stdout)["code"], code)
        self.assertEqual(self.action_count, 0)

    def test_protocol_stall_and_response_stall_are_bounded(self):
        import time
        for command, args, limit in [
            ("health", ("winapp", "health"), 3.5),
            ("get_ui_snapshot", ("winapp", "snapshot", "--timeout-ms", "150"), 1.5)]:
            self.stall_commands = {command}
            started = time.monotonic()
            result = self.cli(*args)
            self.assertLess(time.monotonic() - started, limit)
            self.assertNotEqual(result.returncode, 0)
            payload = json.loads(result.stdout)
            self.assertEqual(payload["stage"], "awaiting_response")

    def test_stale_endpoint_rediscovery_uses_project_daemon(self):
        from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
        import socket
        healthy = json.loads((self.root / ".teshi/cdp-endpoint.json").read_text())
        with socket.socket() as unused:
            unused.bind(("127.0.0.1", 0))
            stale = {**healthy, "ws_url": f"ws://127.0.0.1:{unused.getsockname()[1]}"}
        (self.root / ".teshi/cdp-endpoint.json").write_text(json.dumps(stale))
        requests = []

        class Daemon(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                requests.append((self.path, json.loads(self.rfile.read(int(self.headers["Content-Length"])))))
                body = json.dumps({"ws_url": healthy["ws_url"]}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        daemon = ThreadingHTTPServer(("127.0.0.1", 0), Daemon)
        thread = threading.Thread(target=daemon.serve_forever, daemon=True)
        thread.start()
        try:
            (self.root / ".teshi/daemon.json").write_text(json.dumps({"pid": os.getpid(), "port": daemon.server_port,
                                                                   "started": "2026-10-23T00:00:00Z"}))
            result = self.cli("winapp", "health")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(requests, [("/api/v1/browser/start", {"mode": "winapp"})])
            self.assertEqual(self.action_count, 0)
        finally:
            daemon.shutdown()
            daemon.server_close()
            thread.join(timeout=2)


if __name__ == "__main__":
    unittest.main()
