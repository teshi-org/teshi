"""Offline regressions for Nightly SemVer ordering and release version resolution."""

from contextlib import redirect_stderr, redirect_stdout
import io
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import nightly_version_gate as version_gate
import release as release_script


class NightlyVersionGateTest(unittest.TestCase):
    def test_publishes_when_workspace_version_is_ahead_of_latest_stable(self):
        tags = [
            "v0.2.3.1",
            "v0.7.11",
            "v0.7.13-nightly.20261123.c7d48d0",
            "v0.7.14",
            "v0.7.14-nightly.20261124.b550311",
        ]
        decision, detail = version_gate.evaluate("0.7.15", tags, [])
        self.assertEqual(decision, "publish")
        self.assertIn("0.7.15", detail)
        self.assertIn("v0.7.14", detail)

    def test_fails_closed_when_workspace_version_equals_or_trails_latest_stable(self):
        tags = ["v0.7.11", "v0.7.14", "v0.7.15-nightly.20261125.aaaaaaa"]
        for workspace_version in ("0.7.14", "0.7.13", "0.6.99"):
            with self.subTest(workspace_version=workspace_version):
                decision, detail = version_gate.evaluate(workspace_version, tags, [])
                self.assertEqual(decision, "error")
                self.assertIn("v0.7.14", detail)
                self.assertIn("0.7.15", detail)

    def test_skips_when_head_already_has_stable_or_nightly_tag(self):
        tags = ["v0.7.14"]
        decision, detail = version_gate.evaluate("0.7.14", tags, ["v0.7.14"])
        self.assertEqual(decision, "skip")
        self.assertIn("stable release", detail)

        decision, detail = version_gate.evaluate(
            "0.7.15", tags, ["v0.7.15-nightly.20261125.aaaaaaa"]
        )
        self.assertEqual(decision, "skip")
        self.assertIn("nightly", detail)

    def test_ignores_prerelease_and_non_semver_tags_when_finding_latest_stable(self):
        tags = [
            "v0.2.3.1",
            "v0.7.10",
            "v0.7.15-nightly.20261125.aaaaaaa",
            "v0.8.0-rc.1",
        ]
        self.assertEqual(version_gate.latest_stable_tag(tags), ((0, 7, 10), "v0.7.10"))

    def test_publishes_when_no_stable_tags_exist(self):
        decision, _ = version_gate.evaluate("0.7.15", ["v0.7.15-nightly.20261125.aaaaaaa"], [])
        self.assertEqual(decision, "publish")

    def test_rejects_invalid_workspace_version(self):
        decision, detail = version_gate.evaluate("0.7.15-beta.1", ["v0.7.14"], [])
        self.assertEqual(decision, "error")
        self.assertIn("Invalid workspace version", detail)

    def test_cli_exit_codes_for_publish_skip_error_and_git_failure(self):
        def fake_git(*args):
            if "--points-at" in args:
                return []
            return ["v0.7.14"]

        with patch.object(version_gate, "_git_lines", side_effect=fake_git):
            stdout, stderr = io.StringIO(), io.StringIO()
            with redirect_stdout(stdout), redirect_stderr(stderr):
                code = version_gate.main(["--version", "0.7.15", "--head", "a" * 40])
            self.assertEqual(code, 0)
            self.assertTrue(stdout.getvalue().startswith("publish:"))
            self.assertEqual(stderr.getvalue(), "")

            stdout, stderr = io.StringIO(), io.StringIO()
            with redirect_stdout(stdout), redirect_stderr(stderr):
                code = version_gate.main(["--version", "0.7.14", "--head", "a" * 40])
            self.assertEqual(code, 1)
            self.assertEqual(stdout.getvalue(), "")
            self.assertIn("must be greater than latest stable release v0.7.14", stderr.getvalue())

        with patch.object(
            version_gate,
            "_git_lines",
            side_effect=subprocess.CalledProcessError(1, ["git"], stderr="not a git repo"),
        ):
            stderr = io.StringIO()
            with redirect_stderr(stderr):
                code = version_gate.main(["--version", "0.7.15", "--head", "a" * 40])
            self.assertEqual(code, 1)
            self.assertIn("git tag lookup failed", stderr.getvalue())


class ReleaseVersionResolutionTest(unittest.TestCase):
    def test_determine_release_version_reuses_prebumped_dev_version(self):
        self.assertEqual(release_script.determine_release_version("v0.7.14", "0.7.15", "patch"), "0.7.15")
        self.assertEqual(release_script.determine_release_version("v0.7.14", "0.7.15", "minor"), "0.7.15")
        self.assertEqual(release_script.determine_release_version("v0.7.14", "0.7.14", "patch"), "0.7.15")
        self.assertEqual(release_script.determine_release_version("v0.7.14", "0.7.14", "minor"), "0.7.15")
        self.assertEqual(release_script.determine_release_version("v0.7.14", "0.7.15", "none"), "0.7.15")

    def test_next_dev_version_advances_only_when_not_already_ahead(self):
        self.assertEqual(release_script.next_dev_version("v0.7.14", "0.7.14"), ("0.7.15", True))
        self.assertEqual(release_script.next_dev_version("v0.7.14", "0.7.15"), ("0.7.15", False))
        self.assertEqual(release_script.next_dev_version("v0.7.14", "0.7.13"), ("0.7.15", True))


if __name__ == "__main__":
    unittest.main()
