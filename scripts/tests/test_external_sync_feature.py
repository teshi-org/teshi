"""The partial Feature adapter must fail closed on stale or unbound cases."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "scripts" / "run-tui-acceptance-feature.py"
FEATURE = "features/en-US/external_feature_sync.feature"


class ExternalSyncAdapterTests(unittest.TestCase):
    def check_failure(self, root, scenario, expected):
        request = {"command": "run", "meta": {"project_root": str(root)},
                   "cases": [{"id": "case", "scenario": scenario, "feature_path": FEATURE}]}
        env = os.environ.copy()
        env.pop("TESHI_EXTERNAL_SYNC_TEST_BIN", None)
        result = subprocess.run([sys.executable, str(RUNNER)], input=json.dumps(request),
                                capture_output=True, text=True, encoding="utf-8", env=env)
        events = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual(result.returncode, 1)
        failures = [event for event in events if event["type"] == "case_failed"]
        self.assertEqual(len(failures), 1)
        self.assertIn(expected, failures[0]["error"]["message"])
        self.assertEqual(events[-1]["passed"], 0)

    def test_missing_binary_and_unbound_scenario(self):
        self.check_failure(ROOT, "Discover additions in the configured scope", "must point")
        self.check_failure(ROOT, "Refresh a batch of changes", "not bound")

    def test_modified_source_is_not_executed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / FEATURE
            path.parent.mkdir(parents=True)
            path.write_text("changed", encoding="utf-8")
            self.check_failure(Path(directory), "Discover additions in the configured scope", "source changed")


if __name__ == "__main__":
    unittest.main()
