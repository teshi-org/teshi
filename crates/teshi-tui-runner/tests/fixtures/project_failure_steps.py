"""Source-bound failure acceptance through an owned real terminal."""

import json
import os
from pathlib import Path
import re
import sys
import time
import subprocess

from agent_feature_steps import World as TerminalWorld, emit


def process_exited(pid):
    if os.name != "nt":
        path = Path(f"/proc/{pid}/stat")
        try:
            return path.read_text().rsplit(")", 1)[1].split()[0] == "Z"
        except FileNotFoundError:
            return True
    import ctypes
    from ctypes import wintypes
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    handle = kernel.OpenProcess(0x00100000, False, pid)  # SYNCHRONIZE only
    if not handle:
        assert ctypes.get_last_error() == 87, "cannot inspect the owned child process"
        return True
    try:
        return kernel.WaitForSingleObject(handle, 0) == 0
    finally:
        kernel.CloseHandle(handle)


class World(TerminalWorld):
    def __init__(self, request, case):
        super().__init__(request, case)
        self.project = self.root / "project-a"
        self.project.mkdir()
        self.other = self.root / "project-b"
        self.other.mkdir()
        self.sources = {}
        for project in (self.project, self.other):
            source = f"Feature: {project.name}\n  Scenario: Ready\n    Given the {project.name} step\n"
            (project / "sample.feature").write_text(source, encoding="utf-8")
            self.sources[project.name] = source
        self.evidence = []
        self.blocked = None
        self.chat_active = False
        self.dispatch = None
        self.read_handle = None
        self.read_protected = None
        self.alias = None
        self.persistence_block = None

    def restore_read_access(self):
        if self.read_handle is not None:
            import ctypes
            ctypes.windll.kernel32.CloseHandle(self.read_handle)
            self.read_handle = None
        if self.read_protected is not None:
            self.read_protected.chmod(0o755)
            self.read_protected = None

    def restore_persistence(self):
        if self.persistence_block is not None:
            path, source = self.persistence_block
            path.rmdir()
            path.write_bytes(source)
            self.persistence_block = None

    def block_persistence(self, path):
        self.persistence_block = (path, path.read_bytes())
        path.unlink()
        path.mkdir()

    def wait_file(self, name, timeout=10):
        deadline = time.monotonic() + timeout
        while not (self.root / name).exists():
            assert time.monotonic() < deadline, f"missing fixture file: {name}"
            time.sleep(.01)

    def launch_project(self, acp=False, permission=False, review=False):
        env = self._env(None)
        if acp:
            fixture = Path(__file__).resolve().parents[3] / "teshi-agent-runtime/tests/fixtures/project_switch_agent.py"
            env += [("TESHI_AGENT_BACKEND", "acp"), ("TESHI_ACP_PROGRAM", sys.executable),
                    ("TESHI_ACP_ARGS_JSON", json.dumps([str(fixture), "acp", str(self.root)])),
                    ("TESHI_TEST_AGENT_CHILD", "1")]
            if permission:
                env += [("TESHI_TEST_AGENT_PERMISSION", "1")]
            if review:
                fixture = fixture.with_name("fake_acp_agent.py")
                env += [("TESHI_ACP_ARGS_JSON", json.dumps([str(fixture), "mcp_review"]))]
        program = os.environ["TESHI_SELF_TEST_BIN"]
        args = [str(self.project), "--requirements-root", str(self.requirements)]
        if os.name != "nt" and os.geteuid() == 0:
            args = ["--bounding-set=-dac_override,-dac_read_search", program] + args
            program = "/usr/bin/setpriv"
        self.op("launch", config=dict(program=program, args=args,
                cwd=str(self.root), env=env, cols=140, rows=36))
        self.process = True
        self.expect("Project: project-a")
        self.expect("controlled service" if self.dispatch is not None else "project-a step")

    def step(self, text, table=None):
        if text == "project B contains a Feature with invalid encoding":
            (self.other / "bad.feature").write_bytes(b"\xff\xfe")
            self.read_error_path = "bad.feature"
            return
        if text == "a nested Feature in project B cannot be read":
            nested = self.other / "restricted"
            nested.mkdir()
            path = nested / "hidden.feature"
            path.write_text("Feature: Hidden\n", encoding="utf-8")
            self.read_error_path = "restricted"
            if os.name == "nt":
                import ctypes
                from ctypes import wintypes
                kernel = ctypes.windll.kernel32
                kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                              wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
                kernel.CreateFileW.restype = wintypes.HANDLE
                kernel.CloseHandle.argtypes = [wintypes.HANDLE]
                self.read_handle = kernel.CreateFileW(str(path), 0x80000000, 0, None, 3, 128, None)
                assert self.read_handle != ctypes.c_void_p(-1).value, "cannot lock owned fixture"
            else:
                nested.chmod(0)
                self.read_protected = nested
            return
        if text == "the target read restriction is removed":
            self.restore_read_access()
            return
        if text == "the operator chooses Discard and switch":
            self.expect("Unsaved project state must be resolved first")
            self.op("key", "d")
            return
        if text == "the target read error is visible and A's edit remains available":
            screen = self.expect("Could not prepare project B")
            assert self.read_error_path in screen, screen
            self.expect("Project: project-a")
            self.expect("project-a step unsaved-marker")
            assert "unsaved-marker" not in (self.project / "sample.feature").read_text()
            self.evidence.append(dict(event="target_read_failed_kept_a", path=self.read_error_path, screen=screen))
            return
        if text == "both projects are ready without unsaved edits":
            self.launch_project()
            return
        if text == "recent project history cannot be saved":
            path = self.root / "app-data/recent.json"
            path.parent.mkdir(exist_ok=True)
            path.unlink(missing_ok=True)
            path.mkdir()
            return
        if text == "project B is active with a recent-history persistence warning":
            screen = self.expect("recent history was not saved")
            assert "Project: project-b" in screen, screen
            self.expect("project-b step")
            self.evidence.append(dict(event="history_failure_did_not_undo_activation", screen=screen))
            return
        if text == "recent projects have the same display name and different paths":
            parent = self.root / "other-parent"
            parent.mkdir()
            self.other.rename(parent / "project-a")
            self.other = parent / "project-a"
            self.launch_project()
            history = self.root / "app-data/recent.json"
            history.parent.mkdir(exist_ok=True)
            history.write_text(json.dumps(dict(projects=[dict(root=str(p), display_name="project-a", last_opened_ms=1)
                                                        for p in (self.project, self.other)])), encoding="utf-8")
            return
        if text == "the operator selects the other project's path in the recent picker":
            self.op("click", "Project: project-a")
            screen = self.expect("other-parent")
            compact = re.sub(r"[\s│┃║]", "", screen)
            assert str(self.project) in compact and str(self.other) in compact, screen
            self.evidence.append(dict(event="equal_names_distinct_paths", screen=screen))
            self.op("click", "other")
            return
        if text == "the other project's Feature is active despite the shared name":
            screen = self.expect("project-b step")
            assert "Project: project-a" in screen and "project-a step" not in screen, screen
            self.evidence.append(dict(event="equal_name_pointer_selected_b", screen=screen))
            return
        if text == "both projects' source files remain unchanged":
            for path, name in [(self.project, "project-a"), (self.other, "project-b")]:
                assert (path / "sample.feature").read_text(encoding="utf-8") == self.sources[name]
            return
        if text == "project A has a filesystem directory alias":
            self.alias = self.root / "project-alias"
            if os.name == "nt":
                subprocess.run(["cmd", "/c", "mklink", "/J", str(self.alias), str(self.project)],
                               check=True, capture_output=True)
            else:
                self.alias.symlink_to(self.project, target_is_directory=True)
            return
        if text == "the operator requests project A through its path aliases":
            aliases = [str(self.alias), str(self.project) + os.sep + "." + os.sep, "project-a"]
            if os.name == "nt":
                aliases.append(str(self.project.with_name("PROJECT-A")))
            for alias in aliases:
                self.op("click", "Project: project-a")
                self.expect("Open project")
                self.op("key", "o")
                self.expect("Open project path")
                self.op("write", alias)
                self.op("key", "Enter")
                self.expect("Already on that project")
                self.evidence.append(dict(event="alias_noop", alias=alias, screen=self.screen()))
            return
        if text == "the same-project notice preserves A's unsaved edit without confirmation":
            screen = self.expect("project-a step unsaved-marker")
            assert "Unsaved project state must be resolved first" not in screen, screen
            return
        if text == "project A has an unsaved requirement draft":
            body = "# Boundary requirement\n\nbody\n"
            (self.requirements / "req.md").write_text(body, encoding="utf-8")
            (self.requirements / "_teshi.json").write_text(json.dumps(dict(version=2, store_id="reqstore-boundary",
                documents=[dict(id="boundary", path="req.md", title="Boundary requirement", revision="fixture")])), encoding="utf-8")
            self.launch_project()
            self.op("key", "4")
            self.expect("body")
            self.op("click", "body")
            self.op("key", "i")
            self.op("key", "End")
            self.op("write", " draft-marker")
            self.op("key", "Escape")
            self.expect("body draft-marker")
            return
        if text == "the requirement save destination is unavailable":
            self.block_persistence(self.requirements / "req.md")
            return
        if text == "the operator cancels the failed switch":
            self.op("key", "Escape")
            self.expect("Project: project-a")
            return
        if text == "project A still displays its unsaved requirement draft":
            screen = self.expect("body draft-marker")
            assert self.persistence_block is not None
            self.evidence.append(dict(event="draft_save_failure_retained", screen=screen))
            return
        if text in {"the requirement save destination is restored", "the test-point save destination is restored"}:
            self.restore_persistence()
            return
        if text == "B is active and the requirement draft is saved in the shared store":
            self.expect("Project: project-b")
            source = (self.requirements / "req.md").read_text(encoding="utf-8")
            assert "body draft-marker" in source, source
            assert not (self.other / "req.md").exists()
            self.evidence.append(dict(event="draft_retry_saved_shared_store", source=source))
            return
        if text == "project A has an unsaved test-point field edit":
            path = self.project / "testpoints/testpoints.json"
            path.parent.mkdir()
            path.write_text(json.dumps(dict(version=1, test_points=[dict(id="tp-boundary", title="Boundary point",
                objective="Check persistence", hierarchy_path=["Boundary"], review_state="approved",
                requirement_links=[], scenario_refs=[])])), encoding="utf-8")
            self.launch_project()
            self.op("key", "5")
            self.expect("Boundary point")
            self.op("click", "Title:")
            self.op("write", "界")
            self.expect("Boundary point界")
            return
        if text == "the test-point save destination is unavailable":
            self.block_persistence(self.project / "testpoints/testpoints.json")
            return
        if text == "project A still displays its unsaved test-point edit":
            screen = self.expect("Boundary point界")
            assert self.persistence_block is not None
            self.evidence.append(dict(event="point_save_failure_retained", screen=screen))
            return
        if text == "B is active and A's changed test point still requires human review":
            self.expect("Project: project-b")
            saved = json.loads((self.project / "testpoints/testpoints.json").read_text())
            point = saved["test_points"][0]
            assert point["title"] == "Boundary point界" and point["review_state"] == "proposed", point
            assert not (self.other / "testpoints/testpoints.json").exists()
            self.evidence.append(dict(event="point_retry_saved_without_approval", point=point))
            return
        if text == "the operator undoes and redoes in project B's editor":
            self.op("key", "Escape")
            self.op("key", "1")
            self.expect("project-b step")
            self.op("key", "e")
            self.op("key", "Enter")
            self.op("key", "Ctrl+_")
            time.sleep(.15)
            screen = self.expect("project-b step")
            assert "project-a step" not in screen and "unsaved-marker" not in screen, screen
            self.op("key", "Ctrl+y")
            time.sleep(.15)
            self.expect("project-b step")
            # A positive control proves both shortcuts reached the editor.
            self.op("key", "End")
            self.op("write", "Q")
            self.expect("project-b stepQ")
            self.op("key", "Ctrl+_")
            deadline = time.monotonic() + 5
            while "project-b stepQ" in self.screen():
                assert time.monotonic() < deadline, self.screen()
                time.sleep(.01)
            self.op("key", "Ctrl+y")
            self.expect("project-b stepQ")
            self.op("key", "Ctrl+_")
            time.sleep(.15)
            return
        if text == "project B's editor still contains only project B's content":
            screen = self.expect("project-b step")
            assert "project-a step" not in screen and "unsaved-marker" not in screen, screen
            self.evidence.append(dict(event="editor_history_isolated", screen=screen))
            return
        if text == "project A has proposed test points awaiting human review":
            self.launch_project(acp=True, review=True)
            self.op("key", "3")
            self.expect("Welcome to AI Chat!")
            self.op("key", "Enter")
            self.op("write", "Propose a login test point")
            self.op("key", "Enter")
            self.wait_file("project-a/testpoints/testpoints.json")
            self.expect("Pipeline paused for human review")
            self.op("key", "Escape")
            self.op("key", "5")
            self.expect("Valid login")
            self.review_snapshot = (self.project / "testpoints/testpoints.json").read_bytes()
            points = json.loads(self.review_snapshot)["test_points"]
            assert len(points) == 1 and points[0]["review_state"] == "proposed", points
            self.evidence.append(dict(event="human_review_pending", screen=self.screen()))
            return
        if text == "the operator requests project B through the path picker":
            self.op("click", "Project: project-a")
            self.expect("Open project")
            self.op("key", "o")
            self.expect("Open project path")
            self.op("write", str(self.other))
            self.op("key", "Enter")
            return
        if text == "the operator cancels the busy project switch":
            self.expect("Stop tasks and switch")
            self.op("key", "Escape")
            return
        if text == "project A retains its unapproved test points":
            self.expect("Project: project-a")
            self.expect("Valid login")
            assert (self.project / "testpoints/testpoints.json").read_bytes() == self.review_snapshot
            self.evidence.append(dict(event="review_switch_cancelled", screen=self.screen()))
            return
        if text == "project B becomes active without accepting A's review":
            self.expect("Project: project-b")
            assert (self.project / "testpoints/testpoints.json").read_bytes() == self.review_snapshot
            assert not (self.other / "testpoints/testpoints.json").exists()
            assert not (self.project / "blocked.feature").exists()
            assert not (self.project / "approved.feature").exists()
            self.evidence.append(dict(event="review_switch_completed", screen=self.screen(),
                                      proposed=json.loads(self.review_snapshot)))
            return
        if text == "project A is waiting for ACP tool permission":
            self.launch_project(acp=True, permission=True)
            self.op("key", "3")
            self.expect("Welcome to AI Chat!")
            self.op("key", "Enter")
            self.op("write", "A_PROJECT_ONLY_REQUEST")
            self.op("key", "Enter")
            self.expect("Allow once")
            self.evidence.append(dict(event="permission_pending", screen=self.screen()))
            return
        if text == "the operator clicks the project header behind the permission prompt":
            self.op("click", "Project: project-a")
            return
        if text == "the permission prompt retains project A without approving the tool":
            screen = self.expect("Allow once")
            assert "Open project" not in screen, screen
            assert "Project: project-a" in screen, screen
            trace = (self.root / "agent-evidence/trace.jsonl").read_text(encoding="utf-8")
            assert '"kind": "permission_decision"' not in trace, trace
            self.evidence.append(dict(event="permission_owns_input", screen=screen))
            return
        if text == "the operator rejects the permission request":
            self.op("key", "Escape")
            deadline = time.monotonic() + 5
            while True:
                events = [json.loads(line) for line in (self.root / "agent-evidence/trace.jsonl").read_text(encoding="utf-8").splitlines()]
                decisions = [event for event in events if event["kind"] == "permission_decision"]
                if decisions:
                    assert decisions[-1]["outcome"]["outcome"] == "cancelled", decisions
                    break
                assert time.monotonic() < deadline, "permission rejection was not delivered"
                time.sleep(.01)
            self.evidence.append(dict(event="permission_rejected", decisions=decisions))
            return
        if text == "project B has no stale permission request or old reply":
            screen = self.expect("Project: project-b")
            assert "Allow once" not in screen and "A_LATE_REPLY" not in screen, screen
            events = [json.loads(line) for line in (self.root / "agent-evidence/trace.jsonl").read_text(encoding="utf-8").splitlines()]
            exits = [event for event in events if event["kind"] == "acp_exit" and event.get("project") == "project-a"]
            assert exits and process_exited(exits[-1]["pid"]), events
            self.evidence.append(dict(event="permission_switch_completed", trace=events, screen=screen))
            return
        if text == "isolated projects A and B are open for failure testing":
            return
        if text == "project A has an unsaved Feature edit":
            self.launch_project()
            for key in ("e", "Enter", "End"):
                self.op("key", key)
            self.op("write", " unsaved-marker")
            self.op("key", "Enter")
            self.expect("project-a step unsaved-marker")
            return
        if text == "the Feature save destination is unavailable":
            path = self.project / "sample.feature"
            self.blocked = path.read_bytes()
            path.unlink()
            path.mkdir()
            # The fixture changes the actual filesystem. Keep the dirty buffer
            # through the product's external-change guard before switching.
            self.expect("Feature changed on disk")
            self.op("key", "Escape")
            return
        if text == "the operator requests project B":
            if "Type your message..." in self.screen():
                self.op("click", "Type your message...")
            else:
                self.op("key", "Escape")
                self.op("key", "3")
                self.expect("Welcome to AI Chat!")
                self.op("key", "Enter")
            self.op("write", "/project " + str(self.other))
            self.op("key", "Enter")
            return
        if text == "the operator chooses Save and switch":
            self.expect("Unsaved project state must be resolved first")
            self.op("key", "s")
            return
        if text == "the save error is visible and project A remains active":
            screen = self.expect("Cannot save before switching")
            self.expect("Project: project-a")
            self.evidence.append(dict(event="save_failure", screen=screen))
            return
        if text == "the save destination is restored":
            path = self.project / "sample.feature"
            path.rmdir()
            path.write_bytes(self.blocked)
            self.blocked = None
            return
        if text == "project B becomes active and A's edit is saved":
            self.expect("Project: project-b")
            saved = (self.project / "sample.feature").read_text(encoding="utf-8")
            assert "project-a step unsaved-marker" in saved, saved
            self.evidence.append(dict(event="saved_before_switch", source=saved))
            return
        if text == "project B's source files remain unchanged":
            assert (self.other / "sample.feature").read_text(encoding="utf-8") == self.sources["project-b"]
            return
        if text == "project A has an ACP turn with an active tool process":
            self.launch_project(acp=True)
            self.op("key", "3")
            self.expect("Welcome to AI Chat!")
            self.op("key", "Enter")
            self.op("write", "A_PROJECT_ONLY_REQUEST")
            self.op("key", "Enter")
            self.expect("A_AGENT_ACTIVE")
            self.chat_active = True
            deadline = time.monotonic() + 5
            while not (self.root / "agent-evidence/child-writes.txt").exists():
                assert time.monotonic() < deadline, "tool process did not start"
                time.sleep(.01)
            return
        if text == "the operator chooses Stop tasks and switch":
            self.expect("Stop tasks and switch")
            self.op("key", "s")
            return
        if text == "project B becomes active after the old tool stops":
            self.expect("Project: project-b")
            writes = self.root / "agent-evidence/child-writes.txt"
            before = writes.read_bytes()
            time.sleep(.2)
            assert writes.read_bytes() == before, "old ACP tool kept writing after switching"
            trace = [json.loads(line) for line in (self.root / "agent-evidence/trace.jsonl").read_text().splitlines()]
            assert any(event["kind"] == "acp_exit" for event in trace), trace
            child = next(event for event in trace if event["kind"] == "acp_child_start")
            assert process_exited(child["pid"]), "old tool process is still running"
            self.evidence.append(dict(event="tool_stopped", child_pid=child["pid"], bytes=len(before), trace=trace))
            return
        if text == "no old tool writes appear in project B":
            assert not (self.other / "agent-writes.txt").exists()
            assert (self.other / "sample.feature").read_text(encoding="utf-8") == self.sources["project-b"]
            return
        if text == "project A has a stalled synchronous dispatch":
            (self.project / "sample.feature").write_text(
                '@api\nFeature: project-a\n  Scenario: Ready\n    When [API] I wait for the controlled service\n', encoding="utf-8")
            fixture = Path(__file__).with_name("project_dispatch_fixture.py")
            self.dispatch = subprocess.Popen([sys.executable, str(fixture), str(self.root)],
                                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            self.wait_file("project-a/.teshi/api-endpoint.json")
            self.launch_project()
            self.op("key", "r")
            self.wait_file("dispatch-active")
            return
        if text == "the stop timeout is visible and project A remains usable":
            screen = self.expect("owned task did not exit within")
            self.expect("Project: project-a")
            assert not (self.root / "dispatch-finished").exists()
            self.evidence.append(dict(event="dispatch_timeout_retains_a", screen=screen))
            return
        if text == "the dispatch is allowed to finish":
            (self.root / "dispatch-release").write_text("finish", encoding="utf-8")
            self.wait_file("dispatch-finished")
            return
        if text == "project B becomes active after the dispatch exits":
            if "Stop tasks and switch" in self.screen():
                self.op("key", "s")
            self.expect("Project: project-b")
            assert (self.project / "dispatch-writes.txt").read_text(encoding="utf-8") == "A-only\n"
            assert not (self.other / "dispatch-writes.txt").exists()
            self.evidence.append(dict(event="dispatch_completed_before_switch"))
            return
        if text == "recent project history is empty":
            self.launch_project()
            path = self.root / "app-data/recent.json"
            path.parent.mkdir(exist_ok=True)
            path.write_text(json.dumps(dict(paths=[], projects=[])), encoding="utf-8")
            return
        if text == "the operator opens the project picker from the header":
            self.op("click", "Project: project-a")
            self.expect("Open project")
            return
        if text == "the empty history notice and Open path control are visible":
            self.expect("No recent projects")
            self.expect("[O] Open path")
            self.evidence.append(dict(event="empty_recent_picker", screen=self.screen()))
            return
        if text == "the operator chooses Open path":
            self.op("click", "[O] Open path")
            self.expect("Open project path")
            return
        if text == "the operator enters project B's directory":
            self.op("write", str(self.other))
            self.op("key", "Enter")
            return
        if text == "project B becomes active through the path picker":
            self.expect("Project: project-b")
            self.op("key", "1")
            self.expect("project-b step")
            return
        raise AssertionError("Unbound step: " + text)

    def close(self):
        self.restore_read_access()
        self.restore_persistence()
        if self.alias is not None:
            if os.name == "nt":
                self.alias.rmdir()
            else:
                self.alias.unlink()
            self.alias = None
        # Cooperatively reap the owned fixture even when the old product fails.
        evidence = self.root / "agent-evidence"
        if evidence.exists():
            (evidence / "cleanup-child").write_text("stop", encoding="utf-8")
        if self.blocked is not None:
            path = self.project / "sample.feature"
            path.rmdir()
            path.write_bytes(self.blocked)
        if self.dispatch is not None:
            (self.root / "dispatch-release").write_text("finish", encoding="utf-8")
            self.dispatch.terminate()
            self.dispatch.wait(timeout=10)
        super().close()


def main():
    request = json.load(sys.stdin)
    emit("start_run", total=len(request["cases"]))
    failed = 0
    for case in request["cases"]:
        world = World(request, case)
        artifacts = Path(os.environ.get("TESHI_FEATURE_ARTIFACTS", request["meta"]["artifacts_dir"])) / re.sub(r"[^A-Za-z0-9_-]", "_", case["id"])
        artifacts.mkdir(parents=True, exist_ok=True)
        emit("start_case", case_id=case["id"], name=case["scenario"])
        started = time.monotonic()
        error = None
        try:
            for step in case["steps"]:
                emit("start_step", case_id=case["id"], step_id=str(step["line"]), text=step["text"])
                world.step(step["canonical"])
                emit("end_step", case_id=case["id"], step_id=str(step["line"]), status="passed")
        except Exception as exc:
            error = str(exc)
            emit("end_step", case_id=case["id"], step_id=str(step["line"]), status="failed", message=error)
        finally:
            try:
                (artifacts / "screen.txt").write_text(world.screen(), encoding="utf-8")
                world.op("screenshot", str(artifacts / "screen.svg"))
            finally:
                (artifacts / "evidence.json").write_text(json.dumps(world.evidence, indent=2), encoding="utf-8")
                world.close()
        duration = int((time.monotonic()-started)*1000)
        if error:
            failed += 1
            emit("case_failed", case_id=case["id"], duration_ms=duration, error=dict(message=error))
        else:
            emit("case_passed", case_id=case["id"], duration_ms=duration)
    emit("end_run", passed=len(request["cases"])-failed, failed=failed, skipped=0)
    return bool(failed)


if __name__ == "__main__":
    raise SystemExit(main())
