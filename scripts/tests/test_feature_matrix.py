"""Evidence preservation and per-scenario replay regression tests."""

import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("feature_matrix", Path(__file__).parents[1] / "run-feature-matrix.py")
matrix = importlib.util.module_from_spec(spec)
spec.loader.exec_module(matrix)


class FeatureMatrixTests(unittest.TestCase):
    def test_existing_evidence_directory_is_rejected_without_changes(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            marker = directory / "matrix.json"
            marker.write_text("original failure evidence", encoding="utf-8")
            with patch.object(sys, "argv", ["matrix", "--teshi", sys.executable, "--output-dir", name]):
                with self.assertRaises(SystemExit) as error:
                    matrix.main()
            self.assertEqual(error.exception.code, 2)
            self.assertEqual(marker.read_text(encoding="utf-8"), "original failure evidence")

    def test_browser_cases_dispatch_independently_and_keep_distinct_results(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            feature = root / "case.feature"
            feature.write_text("Feature: Browser\n  Scenario: First\n    Given ready\n  Scenario: Second\n    Given ready\n", encoding="utf-8")
            output = root / "results"
            output.mkdir()
            results = [
                subprocess.CompletedProcess([], 0, '{"event":"replay_complete","steps_replayed":1}', ""),
                subprocess.CompletedProcess([], 1, "", "assertion failed"),
            ]
            with patch.object(matrix, "ROOT", root), patch.dict(matrix.os.environ, {}, clear=True), patch.object(matrix.subprocess, "run", side_effect=results) as run:
                report = matrix.run_browser_cases(feature, ["teshi", "browser", "replay"], output, output / "case.json", output / "case.log", False, None)
            self.assertEqual([call.args[0][-2:] for call in run.call_args_list], [["--scenario", "First"], ["--scenario", "Second"]])
            self.assertEqual([case["status"] for case in report["cases"]], ["passed", "failed"])
            self.assertEqual(report["summary"]["failed"], 1)
            self.assertEqual(report["summary"]["passed"], 1)
            self.assertEqual(len(list(output.glob("*-f0_s*.log"))), 2)

    def test_generic_browser_session_text_is_not_an_environment_blocker(self):
        self.assertFalse(matrix.is_blocked_output("browser session assertion failed"))
        self.assertTrue(matrix.is_blocked_output("browser target unavailable"))


if __name__ == "__main__":
    unittest.main()
