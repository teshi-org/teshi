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
        self.response = {"ok": True, "bounds": {"x": 20, "y": 30, "width": 2, "height": 3}, "dpi": 144}
        logger = logging.getLogger("winapp-test-sidecar")
        logger.setLevel(logging.CRITICAL)

        def handler(socket):
            command = json.loads(socket.recv())
            self.commands.append(command)
            if self.disconnect:
                return
            if command["cmd"] == "get_target":
                response = {"ok": True, "target": {"attached": True}}
            elif command["cmd"] == "screenshot":
                response = {"ok": True, "screenshot": "AA=="}
            elif command.get("action") in (
                "click", "pointer_click", "fill", "assert_visible", "assert_not_exists", "assert_text"
            ):
                response = {"ok": True}
            else:
                response = self.response
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
        self.assertEqual(replay.returncode, 0, replay.stderr)
        pointer_commands = [
            command for command in self.commands
            if command.get("action") == "pointer_click"
        ]
        self.assertTrue(pointer_commands)
        self.assertEqual(pointer_commands[-1]["action"], "pointer_click")
        self.assertEqual(pointer_commands[-1]["mode"], "auto")
        assertion_commands = [
            command for command in self.commands
            if command.get("action") == "assert_visible"
        ]
        self.assertTrue(assertion_commands)
        self.assertEqual(assertion_commands[-1]["mode"], "auto")

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


if __name__ == "__main__":
    unittest.main()
