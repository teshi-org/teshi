"""Fail-closed coverage tests: missing, duplicated, skipped and mismatched evidence."""

from copy import deepcopy
import json
import hashlib
import io
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import ci_suite as suite


class PartitionTests(unittest.TestCase):
    def test_failed_binary_does_not_hide_results_from_independent_later_binaries(self):
        identity = dict(source_sha="local", platform=sys.platform, partition_sha256="partition")
        targets = [dict(id=f"test:{name}", owner="terminal-b", executable=f"{name}.exe",
                        sha256="binary", tests=[dict(name=name, ignored=True)])
                   for name in ("first", "second")]
        inventory = dict(**identity, targets=targets, doc_tests=[])
        labels = []

        def run(command, artifacts, label, timeout, serial_libtest=False):
            labels.append(label)
            failed = label == "test:first"
            suite.write(artifacts / "timings.json", dict(tests=[dict(name=label.split(":")[1], result="FAILED" if failed else "ok")]))
            return 101 if failed else 0

        with tempfile.TemporaryDirectory() as name, \
                patch.object(suite, "assert_identity"), \
                patch.object(suite, "configure_environment"), \
                patch.object(suite, "identity", return_value=identity), \
                patch.object(suite, "digest", return_value="binary"), \
                patch.object(suite.command_runner, "run", side_effect=run):
            with self.assertRaises(RuntimeError):
                suite.execute(inventory, "terminal-b", Path(name))
            result = suite.read(Path(name) / "execution.json")
            self.assertEqual(labels, ["test:first", "test:second"])
            self.assertEqual(len(result["targets"]), 2)
            self.assertEqual(result["targets"][1]["command_status"], 0)
            self.assertEqual(result["status"], "failed")
            self.assertTrue(result["errors"])

    def test_current_modules_have_exactly_one_owner(self):
        self.assertEqual(len(suite.ownership()), len(list((suite.ROOT / "crates/teshi-tui-runner/tests").glob("*.rs"))))

    def test_new_missing_duplicate_and_unknown_modules_are_rejected(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            modules = root / "crates/teshi-tui-runner/tests"
            modules.mkdir(parents=True)
            for module in ("one", "two"):
                (modules / f"{module}.rs").touch()
            spec = dict(shards={"terminal-a": ["one"], "terminal-b": ["two"]})
            self.assertEqual(suite.ownership(root, spec)["one"], "terminal-a")
            for a, b in (([], ["two"]), (["one"], ["one", "two"]), (["one", "extra"], ["two"])):
                with self.subTest(a=a, b=b), self.assertRaises(ValueError):
                    suite.ownership(root, dict(shards={"terminal-a": a, "terminal-b": b}))

    def test_ignored_tests_must_actually_run_in_their_shard(self):
        target = dict(id="test:fixture", tests=[dict(name="normal", ignored=False), dict(name="acceptance", ignored=True)])
        suite.assert_observed(target, [dict(name="normal", result="ok"), dict(name="acceptance", result="ignored, requires TUI")], "backend")
        suite.assert_observed(target, [dict(name="acceptance", result="ok")], "terminal-a")
        for observations in ([], [dict(name="acceptance", result="ignored")],
                             [dict(name="acceptance", result="ok")] * 2,
                             [dict(name="normal", result="ok")]):
            with self.subTest(observations=observations), self.assertRaises(ValueError):
                suite.assert_observed(target, observations, "terminal-a")

    def test_manual_external_acceptance_is_explicitly_deferred_and_never_counted_as_a_pass(self):
        target = dict(id="test:fixture", owner="terminal-a", tests=[dict(name="required", ignored=True), dict(name="external", ignored=True)])
        policy = {"test:fixture::external": "authenticated external provider"}
        with patch.object(suite, "manual_tests", return_value=policy):
            suite.assert_observed(target, [dict(name="required", result="ok")], "terminal-a")
            with self.assertRaises(ValueError):
                suite.assert_observed(target, [dict(name="required", result="ok"), dict(name="external", result="ok")], "terminal-a")
            deferred = suite.deferred_tests(dict(targets=[target]), "terminal-a")
            self.assertEqual(deferred, [dict(binary="test:fixture", name="external", status="not-run", reason="authenticated external provider")])


class EvidenceFixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.plan = [dict(scenario="outline", line_number=3, variants=[
            dict(attempt=i, example={"value": text}, steps=[dict(line=2, text="background"), dict(line=4, text=f"check {text}")])
            for i, text in ((1, "first"), (2, "second"))])]
        self.directory = self.root / "directory/directory"
        self.directory.mkdir(parents=True)
        suite.write(self.directory / "request.json", dict(cases=[dict(id="f0:s0", scenario="outline", line_number=3)]))
        captures = self.directory / "captures"
        captures.mkdir()
        self.events = []
        for variant in self.plan[0]["variants"]:
            for step in variant["steps"]:
                common = dict(case_id="f0:s0", step_id=str(step["line"]), attempt=variant["attempt"])
                self.events.append(dict(common, type="start_step", text=step["text"], example=variant["example"]))
                self.events.append(dict(common, type="end_step", status="passed"))
                stem = f"{variant['attempt']}-{step['line']}"
                files = dict(screenshot_file=f"{stem}.png", svg_file=f"{stem}.svg", text_file=f"{stem}.txt")
                (captures / files["screenshot_file"]).write_bytes((suite.ROOT / "crates/teshi-ui/tests/fixtures/replay-first.png").read_bytes())
                (captures / files["svg_file"]).write_text('<svg xmlns="http://www.w3.org/2000/svg"></svg>')
                (captures / files["text_file"]).write_text("viewport")
                self.events.append(dict(common, type="step_screenshot", step_line=step["line"], status="passed", capture_status="available", **files))
        self.events.append(dict(type="end_run", examples_passed=2, examples_failed=0, passed=1, failed=0))
        self.save_events()
        self.rows = [dict(case_id="f0:s0", attempt=v["attempt"], example=v["example"], status="passed") for v in self.plan[0]["variants"]]
        suite.write(self.directory / "timing-summary.json", self.rows)
        suite.write(self.directory / "report.json", dict(status="passed", summary=dict(passed=1, total=1)))
        self.identity = dict(source_sha="a" * 40, platform=sys.platform, partition_sha256="partition")
        self.target = dict(id="test:fixture", owner="terminal-a", tests=[dict(name="normal", ignored=False), dict(name="acceptance", ignored=True)])
        self.inventory = dict(**self.identity, targets=[self.target], directory_plan=self.plan, doc_tests=[])
        for shard in ("backend", "terminal-a", "terminal-b", "directory"):
            targets = []
            if shard in ("backend", "terminal-a"):
                tests = [dict(name="acceptance", result="ignored" if shard == "backend" else "ok")]
                if shard == "backend":
                    tests.append(dict(name="normal", result="ok"))
                targets.append(dict(id="test:fixture", tests=tests, command_status=0))
            suite.write(self.root / shard / "execution.json", dict(**self.identity, shard=shard, status="passed", targets=targets, doctests=[], inventory_sha256=suite.inventory_digest(self.inventory)))

    def save_events(self):
        (self.directory / "binding-events.ndjson").write_text("\n".join(json.dumps(event) for event in self.events), encoding="utf-8")

    def test_gate_accepts_complete_platform_evidence(self):
        with patch.object(suite, "identity", return_value=self.identity):
            suite.gate(self.inventory, self.root)

    def test_gate_rejects_missing_skipped_failed_or_wrong_sha_shards(self):
        path = self.root / "terminal-b/execution.json"
        original = suite.read(path)
        for change in (dict(status="skipped"), dict(status="failed"), dict(source_sha="b" * 40), dict(platform="other"), dict(inventory_sha256="stale inventory")):
            suite.write(path, dict(original, **change))
            with self.subTest(change=change), patch.object(suite, "identity", return_value=self.identity), self.assertRaises(ValueError):
                suite.gate(self.inventory, self.root)
        path.unlink()
        with patch.object(suite, "identity", return_value=self.identity), self.assertRaises(FileNotFoundError):
            suite.gate(self.inventory, self.root)

    def test_gate_rejects_a_missing_runtime_test_even_when_command_succeeded(self):
        path = self.root / "terminal-a/execution.json"
        result = suite.read(path)
        result["targets"][0]["tests"].clear()
        suite.write(path, result)
        with patch.object(suite, "identity", return_value=self.identity), self.assertRaises(ValueError):
            suite.gate(self.inventory, self.root)

    def test_directory_missing_duplicate_and_failed_steps_cannot_pass(self):
        original = deepcopy(self.events)
        for change in ("missing", "duplicate", "failed", "substitution"):
            self.events = deepcopy(original)
            if change == "missing":
                self.events.pop(1)
            elif change == "duplicate":
                self.events.insert(0, self.events[0])
            elif change == "failed":
                self.events[1]["status"] = "failed"
            else:
                self.events[0]["text"] = "cached result"
            self.save_events()
            with self.subTest(change=change), self.assertRaises(ValueError):
                suite.verify_directory(self.plan, self.directory)

    def test_missing_example_result_and_fake_capture_files_cannot_pass(self):
        suite.write(self.directory / "timing-summary.json", self.rows[:1])
        with self.assertRaises(ValueError):
            suite.verify_directory(self.plan, self.directory)
        suite.write(self.directory / "timing-summary.json", self.rows)
        image = next((self.directory / "captures").glob("*.png"))
        image.write_text("not an image")
        with self.assertRaises(ValueError):
            suite.verify_directory(self.plan, self.directory)


class PreparationCacheTests(unittest.TestCase):
    def test_archive_cannot_escape_the_checkout_or_restore_links(self):
        for unsafe in ("path", "link"):
            with self.subTest(unsafe=unsafe), tempfile.TemporaryDirectory() as name:
                base = Path(name)
                root = base / "checkout"
                root.mkdir()
                archive_path = base / "bundle.tar.gz"
                member_name = "../outside" if unsafe == "path" else "target/debug/link"
                with tarfile.open(archive_path, "w:gz") as archive:
                    member = tarfile.TarInfo(member_name)
                    if unsafe == "link":
                        member.type = tarfile.SYMTYPE
                        member.linkname = "../../outside"
                    archive.addfile(member, io.BytesIO(b""))
                identity = dict(source_sha="a", platform=sys.platform, partition_sha256="b")
                inventory = dict(**identity, build_root=str(root), files={member_name: hashlib.sha256(b"").hexdigest()})
                with patch.object(suite, "ROOT", root), patch.object(suite, "identity", return_value=identity), self.assertRaises(ValueError):
                    suite.unpack(inventory, archive_path)
                self.assertFalse((base / "outside").exists())

    def test_archive_is_verified_before_overwriting_a_compiled_artifact(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            binary = root / "target/debug/fixture"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"original")
            archive_path = root / "binaries.tar.gz"
            data = b"corrupted artifact"
            with tarfile.open(archive_path, "w:gz") as archive:
                member = tarfile.TarInfo("target/debug/fixture")
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))
            identity = dict(source_sha="a", platform=sys.platform, partition_sha256="b")
            inventory = dict(**identity, build_root=str(root), files={"target/debug/fixture": hashlib.sha256(b"expected").hexdigest()})
            with patch.object(suite, "ROOT", root), patch.object(suite, "identity", return_value=identity), self.assertRaises(ValueError):
                suite.unpack(inventory, archive_path)
            self.assertEqual(binary.read_bytes(), b"original")

    def test_binary_hash_cache_invalidates_changed_input_and_uses_bounded_entries(self):
        from test_ci_process import bindings
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "executable"
            path.write_bytes(b"first")
            first = bindings.binary_identity(path)
            with patch.object(bindings.hashlib, "sha256", side_effect=AssertionError("rehashed unchanged executable")):
                self.assertEqual(bindings.binary_identity(path), first)
            path.write_bytes(b"different input")
            self.assertNotEqual(bindings.binary_identity(path), first)
            self.assertEqual(len(bindings._BINARY_IDENTITIES), 1)


if __name__ == "__main__":
    unittest.main()
