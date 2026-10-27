"""Owned-instance and checker contracts; no native windows or input required."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import winapp_service as service


class ExplorationContractTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        # These contracts need no overlay, capture device, COM, or desktop.
        self.session = object.__new__(service.WinAppSession)
        self.session.project_root = Path(self.temp.name)
        self.session.instances = {}

    def test_unknown_token_never_falls_back_to_pid(self):
        with patch.object(service, "windows_of_pid") as windows:
            result = self.session.instance_command(
                {"action": "stop", "instance": "lost", "pid": 123})
        self.assertEqual(result["code"], "instance_unknown")
        windows.assert_not_called()

    def test_launch_owns_handle_and_status_does_not_assert_business_success(self):
        process = Mock(pid=123)
        process.poll.return_value = None
        with patch.object(service.subprocess, "Popen", return_value=process) as launch, \
                patch.object(service, "windows_of_pid", return_value=[]):
            result = self.session.instance_command({"action": "launch", "argv": ["app", "a b"]})
        self.assertIs(self.session.instances[result["instance"]], process)
        self.assertEqual(launch.call_args.args[0], ["app", "a b"])
        self.assertFalse(launch.call_args.kwargs["shell"])
        self.assertEqual(result["business_result"], "unverified")

    def test_exited_handle_cannot_claim_windows_of_a_reused_pid(self):
        process = Mock(pid=123)
        process.poll.return_value = 0
        self.session.instances["owned"] = process
        with patch.object(service, "windows_of_pid") as windows:
            result = self.session.instance_command(
                {"action": "status", "instance": "owned", "expected_windows": 1})
        windows.assert_not_called()
        self.assertFalse(result["ok"])
        self.assertEqual(result["windows"], [])

    def test_stop_timeout_is_unverified_and_not_retry_safe(self):
        process = Mock(pid=123)
        process.poll.return_value = None
        process.wait.side_effect = subprocess.TimeoutExpired("app", 1)
        self.session.instances["owned"] = process
        result = self.session.instance_command({"action": "stop", "instance": "owned"})
        process.terminate.assert_called_once()
        self.assertEqual(result["business_result"], "unverified")
        self.assertFalse(result["retry_safe"])

    def check(self, result, exit_code=0):
        script = "import json,sys; request=json.load(sys.stdin); " \
                 + f"print(json.dumps({result!r})); sys.exit({exit_code})"
        response = self.session.external_check(
            {"argv": [sys.executable, "-c", script], "input": {"sample": 1}})
        evidence = Path(response["evidence"])
        self.assertTrue(evidence.is_relative_to(self.session.project_root))
        self.assertTrue(evidence.is_file())
        self.assertEqual(json.loads(evidence.read_text())["business_result"], response["business_result"])
        return response

    def test_checker_requires_calibrated_thresholds_for_pass(self):
        result = self.check({"business_result": "passed"})
        self.assertFalse(result["ok"])
        self.assertEqual(result["code"], "invalid_checker_result")

    def test_checker_calibrated_pass(self):
        self.assertTrue(self.check({"business_result": "passed", "thresholds_verified": True})["ok"])

    def test_checker_nonzero_exit_overrides_pass(self):
        result = self.check({"business_result": "passed", "thresholds_verified": True}, 3)
        self.assertFalse(result["ok"])
        self.assertEqual(result["business_result"], "failed")

    def test_checker_blocked_is_preserved(self):
        result = self.check({"business_result": "blocked"})
        self.assertFalse(result["ok"])
        self.assertEqual(result["business_result"], "blocked")

    def test_checker_timeout_saves_evidence(self):
        result = self.session.external_check({"argv": [sys.executable, "-c", "import time; time.sleep(10)"],
                                              "check_timeout_ms": 20})
        self.assertEqual(result["code"], "checker_timeout")
        self.assertFalse(result["retry_safe"])
        self.assertTrue(Path(result["evidence"]).is_file())


if __name__ == "__main__":
    unittest.main()
