"""Experiment comparison must reject incomplete or mismatched evidence."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("compare_temp", ROOT / "scripts/compare-ci-temp.py")
comparison = importlib.util.module_from_spec(spec)
spec.loader.exec_module(comparison)


class ComparisonTests(unittest.TestCase):
    def fixture(self, root):
        for mode in ("default", "runner"):
            env = root / mode / "environment/temp-environment.json"
            env.parent.mkdir(parents=True)
            env.write_text(json.dumps(dict(source_sha="a" * 40, runner=dict(image_version="same-image"))))
            for repeat in range(1, 4):
                for workload in ("directory", "management", "terminal"):
                    result = root / mode / f"repeat-{repeat}/{workload}/timings.json"
                    result.parent.mkdir(parents=True)
                    result.write_text(json.dumps(dict(returncode=0, timed_out=False, cleanup_error=None,
                        elapsed_seconds=repeat * (2 if mode == "default" else 1))))

    def test_three_complete_same_sha_runs_use_medians(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            self.fixture(root)
            report = comparison.compare(root)
            self.assertIn("| directory | 4.000s | 2.000s | 2.000x |", report)

    def test_missing_failed_or_mismatched_runs_are_not_reported_as_speedups(self):
        for failure in ("missing", "failed", "sha", "image"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                self.fixture(root)
                result = root / "runner/repeat-3/directory/timings.json"
                if failure == "missing":
                    result.unlink()
                elif failure == "failed":
                    record = json.loads(result.read_text())
                    record["returncode"] = 1
                    result.write_text(json.dumps(record))
                else:
                    env = root / "runner/environment/temp-environment.json"
                    record = json.loads(env.read_text())
                    if failure == "sha":
                        record["source_sha"] = "b" * 40
                    else:
                        record["runner"]["image_version"] = "different-image"
                    env.write_text(json.dumps(record))
                with self.assertRaises((FileNotFoundError, ValueError)):
                    comparison.compare(root)

    @unittest.skipUnless(os.name == "nt", "Windows runtime path resolution")
    def test_runner_override_reaches_fresh_python_and_rust_processes(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            env_file = root / "github-env"
            env_file.touch()
            environment = dict(os.environ, RUNNER_TEMP=str(root), GITHUB_ENV=str(env_file))
            result = subprocess.run(["pwsh", "-NoProfile", "-File", str(ROOT / "scripts/prepare-ci-temp.ps1"),
                "-Mode", "runner", "-Evidence", str(root / "evidence")], env=environment,
                text=True, encoding="utf-8", capture_output=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            record = json.loads((root / "evidence/temp-environment.json").read_text(encoding="utf-8-sig"))
            self.assertEqual(Path(record["python_temp"]), Path(record["rust_temp"]))
            self.assertTrue(Path(record["python_temp"]).is_relative_to(root))
            self.assertIn("TEMP=" + record["python_temp"], env_file.read_text(encoding="utf-8-sig"))


if __name__ == "__main__":
    unittest.main()
