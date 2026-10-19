"""Portable project-switch fixtures and source-bound template validation."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location(
    "tui_sidecar", Path(__file__).parents[1] / "run-tui-sidecar-feature.py"
)
adapter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(adapter)


class ProjectSwitchTemplateTests(unittest.TestCase):
    def test_localized_project_switch_templates_keep_the_same_actions(self):
        templates = [
            adapter.ROOT / "features/en-US/tui_project_switch.tui.template.json",
            adapter.ROOT / "features/zh-CN/项目切换.tui.template.json",
        ]
        actions = [
            {step["line"]: step["actions"] for step in json.loads(path.read_text(encoding="utf-8"))["steps"]}
            for path in templates
        ]
        self.assertEqual(actions[0].keys(), actions[1].keys())
        for line in actions[0]:
            with self.subTest(line=line):
                self.assertEqual(actions[0][line], actions[1][line])

    def test_session_evidence_rejects_legacy_assignment_and_context_leaks(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / "requirements").mkdir()
            adapter.configure_session_fixture(root, "history")
            adapter.verify_session_fixture(root, "history", {})
            path = root / "sessions/legacy.json"
            original = path.read_bytes()
            session = json.loads(original)
            session["project_root"] = str(root / "project-a")
            path.write_text(json.dumps(session))
            with self.assertRaisesRegex(ValueError, "ownership changed"):
                adapter.verify_session_fixture(root, "history", {})
            path.write_bytes(original)
            path = root / "sessions/session-b.json"
            session = json.loads(path.read_text())
            session["messages"].append({"role": "Assistant", "content": "SESSION_A_REPLY"})
            path.write_text(json.dumps(session))
            with self.assertRaisesRegex(ValueError, "context leaked"):
                adapter.verify_session_fixture(root, "history", {})

    def test_association_evidence_rejects_changed_global_or_local_sources(self):
        for relative in ("requirements/shared.md", "project-b/testpoints/testpoints.json"):
            with self.subTest(relative=relative), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                for directory in ("requirements", "project-a", "project-b"):
                    (root / directory).mkdir()
                sources = adapter.configure_session_fixture(root, "links")
                adapter.verify_session_fixture(root, "links", sources)
                path = root / relative
                path.write_bytes(path.read_bytes() + b"\n")
                with self.assertRaisesRegex(ValueError, "mutated"):
                    adapter.verify_session_fixture(root, "links", sources)

    def test_native_evidence_rejects_an_open_stream_and_cross_project_context(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            evidence = root / "agent-evidence"
            evidence.mkdir()
            events = [
                {"kind": "native_prompt", "project": "project-a", "text": "A_PROJECT_ONLY_REQUEST"},
                {"kind": "native_prompt", "project": "project-b", "text": "B_PROJECT_ONLY_REQUEST", "a_stream_closed": False},
            ]
            trace = evidence / "trace.jsonl"
            trace.write_text("\n".join(json.dumps(item) for item in events), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "still open"):
                adapter.verify_agent_shutdown(root, "native")
            events[1].update(a_stream_closed=True, text="B_PROJECT_ONLY_REQUEST A_PROJECT_ONLY_REQUEST")
            trace.write_text("\n".join(json.dumps(item) for item in events), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "conversation context"):
                adapter.verify_agent_shutdown(root, "native")

    def test_acp_evidence_rejects_a_process_with_another_project_cwd(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            evidence = root / "agent-evidence"
            evidence.mkdir()
            events = [
                {"kind": "acp_start", "pid": 1, "cwd": str(root / "project-a")},
                {"kind": "acp_start", "pid": 2, "cwd": str(root / "project-a")},
                {"kind": "acp_prompt", "pid": 1, "project": "project-a", "text": "A_PROJECT_ONLY_REQUEST"},
                {"kind": "acp_prompt", "pid": 2, "project": "project-b", "text": "B_PROJECT_ONLY_REQUEST"},
                {"kind": "acp_exit", "pid": 1, "at": 1},
                {"kind": "acp_session", "pid": 2, "at": 2},
            ]
            (evidence / "trace.jsonl").write_text("\n".join(json.dumps(item) for item in events), encoding="utf-8")
            with patch.object(adapter, "process_running", return_value=False):
                with self.assertRaisesRegex(ValueError, "cwd belongs to another project"):
                    adapter.verify_agent_shutdown(root, "acp")

    def runner_records(self, root):
        for project, roles in (("project-a", ("runner", "child")), ("project-b", ("runner",))):
            evidence = root / project / "runner-evidence"
            evidence.mkdir(parents=True)
            for role in roles:
                (evidence / f"{project}-{role}.json").write_text(json.dumps({
                    "cwd": str(root / project), "project": project, "pid": 1,
                    "process_identity": {"pid": 1, "platform": "windows", "creation_time": "100"},
                }))
                if project == "project-a":
                    (evidence / f"{project}-{role}.writes").write_text("0\n1\n")

    def test_shutdown_evidence_rejects_a_child_that_keeps_writing(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            self.runner_records(root)
            child = root / "project-a/runner-evidence/project-a-child.writes"
            with patch.object(adapter.time, "sleep", side_effect=lambda _: child.write_text("0\n1\n2\n")):
                with self.assertRaisesRegex(ValueError, "not quiescent"):
                    adapter.verify_runner_shutdown(root)

    def test_shutdown_evidence_checks_b_working_directory(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            self.runner_records(root)
            record = root / "project-b/runner-evidence/project-b-runner.json"
            record.write_text(json.dumps({"cwd": str(root / "project-a")}))
            with patch.object(adapter.time, "sleep"):
                with self.assertRaisesRegex(ValueError, "another project's working directory"):
                    adapter.verify_runner_shutdown(root)

    def test_shutdown_evidence_rejects_a_live_process_with_stable_files(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            self.runner_records(root)
            with patch.object(adapter.time, "sleep"), patch.object(adapter, "process_running", return_value=True):
                with self.assertRaisesRegex(ValueError, "still alive"):
                    adapter.verify_runner_shutdown(root)

    def test_process_identity_distinguishes_a_reused_pid_without_signaling_it(self):
        expected = {"pid": 123, "platform": "windows", "creation_time": "100"}
        for created, running in (("100", True), ("101", False)):
            evidence = {}
            observed = dict(expected, creation_time=created, running=True)
            with patch.object(adapter, "process_identity", return_value=observed):
                self.assertEqual(adapter.process_running(123, expected, evidence), running)
            self.assertEqual(evidence["state"], "running" if running else "pid_reused")
            self.assertEqual(evidence["observed_identity"], observed)

    def test_process_identity_accepts_only_known_exit_or_absence(self):
        expected = {"pid": 123, "platform": "windows", "creation_time": "100"}
        for observed in (None, dict(expected, running=False, exit_time="200")):
            with patch.object(adapter, "process_identity", return_value=observed):
                self.assertFalse(adapter.process_running(123, expected))
        for expected_record in ({}, dict(expected, pid=456), dict(expected, creation_time="invalid")):
            with self.assertRaisesRegex(ValueError, "inconsistent recorded"):
                adapter.process_running(123, expected_record)
        evidence = {}
        with patch.object(adapter, "process_identity", side_effect=PermissionError("probe denied")):
            with self.assertRaisesRegex(ValueError, "probe denied"):
                adapter.process_running(123, expected, evidence)
        self.assertEqual(evidence["state"], "unknown")

    def test_shutdown_failure_retains_recorded_and_observed_live_identity(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            self.runner_records(root)
            path = root / "artifacts/runner-shutdown.json"
            observed = {"pid": 1, "platform": "windows", "creation_time": "100", "running": True}
            with patch.object(adapter.time, "sleep"), patch.object(adapter, "process_identity", return_value=observed):
                with self.assertRaisesRegex(ValueError, "still alive"):
                    adapter.verify_runner_shutdown(root, path)
            evidence = json.loads(path.read_text())
            self.assertTrue(evidence["writes_stopped"])
            self.assertNotIn("a_processes_exited", evidence)
            self.assertEqual(set(evidence["process_probes"]), {"a_runner", "a_child"})
            self.assertEqual(evidence["process_probes"]["a_child"]["observed_identity"], observed)
            self.assertEqual(evidence["process_probes"]["a_runner"]["state"], "running")

    def test_shutdown_distinguishes_reused_pid_and_retains_that_fact(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            self.runner_records(root)
            observed = {"pid": 1, "platform": "windows", "creation_time": "101", "running": True}
            with patch.object(adapter.time, "sleep"), patch.object(adapter, "process_identity", return_value=observed):
                evidence = adapter.verify_runner_shutdown(root)
            self.assertTrue(evidence["a_processes_exited"])
            self.assertTrue(all(probe["state"] == "pid_reused" for probe in evidence["process_probes"].values()))

    def test_shutdown_missing_or_unreadable_identity_fails_and_keeps_evidence(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            self.runner_records(root)
            path = root / "artifacts/runner-shutdown.json"
            with patch.object(adapter.time, "sleep"), patch.object(adapter, "process_identity", side_effect=PermissionError("probe denied")):
                with self.assertRaisesRegex(ValueError, "probe denied"):
                    adapter.verify_runner_shutdown(root, path)
            self.assertEqual(json.loads(path.read_text())["process_probes"]["a_runner"]["state"], "unknown")
            record_path = root / "project-a/runner-evidence/project-a-runner.json"
            record = json.loads(record_path.read_text())
            record.pop("process_identity")
            record_path.write_text(json.dumps(record))
            with patch.object(adapter.time, "sleep"):
                with self.assertRaisesRegex(ValueError, "no recorded process creation identity"):
                    adapter.verify_runner_shutdown(root, path)
            self.assertEqual(json.loads(path.read_text())["process_probes"]["a_runner"]["state"], "unknown")

    def test_real_owned_process_identity_survives_exit_detection(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"],
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            identity = adapter.process_identity(child.pid)
            self.assertEqual(identity["pid"], child.pid)
            self.assertTrue(identity["running"])
            self.assertTrue(adapter.process_running(child.pid, identity))
            self.assertEqual(adapter.process_identity(os.getpid())["pid"], os.getpid())
            child.terminate()  # The Popen handle belongs to this test, never a discovered PID.
            child.wait(timeout=5)
            self.assertFalse(adapter.process_running(child.pid, identity))
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=5)

    def test_absolute_cli_paths_stay_relative_inside_fixtures(self):
        with tempfile.TemporaryDirectory() as name:
            project = Path(name)
            source = project / "features/case.feature"
            for requested in (str(source), "features/case.feature"):
                original, relative = adapter.fixture_feature_paths(project, requested)
                self.assertEqual(original, source.resolve())
                self.assertEqual(relative, Path("features/case.feature"))
                self.assertFalse(relative.is_absolute())
            with self.assertRaises(ValueError):
                adapter.fixture_feature_paths(project, str(project.parent / "outside.feature"))

    def test_each_locale_expands_into_independent_fixture_roots(self):
        for locale, stem in (("en-US", "tui_project_switch"), ("zh-CN", "项目切换")):
            feature = adapter.ROOT / "features" / locale / (stem + ".feature")
            source = feature.read_text(encoding="utf-8")
            for _ in range(2):
                with self.subTest(locale=locale), tempfile.TemporaryDirectory() as name:
                    root = Path(name)
                    binary = str(root / "checkout-teshi")
                    sidecar = adapter.project_switch_sidecar(
                        source, feature.with_suffix(".tui.template.json"), root, binary
                    )
                    config = sidecar["steps"][0]["actions"][0]["config"]
                    self.assertEqual(config["program"], binary)
                    self.assertEqual(Path(config["args"][0]), root / "project-a")
                    self.assertTrue(all(Path(value).is_relative_to(root) for _, value in config["env"]))
                    self.assertNotIn("${", json.dumps(sidecar))
                    self.assertIn("project-b step", (root / "project-b/sample.feature").read_text())
                    self.assertEqual((root / "project-invalid/teshi.toml").read_text(), "[runner\n")

    def test_changed_source_text_or_lines_fail_before_fixture_creation(self):
        feature = adapter.ROOT / "features/en-US/tui_project_switch.feature"
        source = feature.read_text(encoding="utf-8")
        for changed in ("\n" + source, source.replace("the isolated projects are ready", "unbound setup")):
            with tempfile.TemporaryDirectory() as name:
                root = Path(name)
                with self.assertRaisesRegex(ValueError, "stale project-switch template"):
                    adapter.project_switch_sidecar(
                        changed, feature.with_suffix(".tui.template.json"), root, "teshi"
                    )
                self.assertEqual(list(root.iterdir()), [])

    def test_unknown_token_fails_before_fixture_creation(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            template = root / "case.tui.template.json"
            template.write_text(json.dumps({"steps": [], "unknown": "${UNDECLARED}"}))
            with self.assertRaisesRegex(ValueError, "unknown project-switch template token"):
                adapter.project_switch_sidecar("", template, root, "teshi")
            self.assertEqual(list(root.iterdir()), [template])


if __name__ == "__main__":
    unittest.main()
