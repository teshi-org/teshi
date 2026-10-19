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

    def wait_file(self, name, timeout=10):
        deadline = time.monotonic() + timeout
        while not (self.root / name).exists():
            assert time.monotonic() < deadline, f"missing fixture file: {name}"
            time.sleep(.01)

    def launch_project(self, acp=False):
        env = self._env(None)
        if acp:
            fixture = Path(__file__).resolve().parents[3] / "teshi-agent-runtime/tests/fixtures/project_switch_agent.py"
            env += [("TESHI_AGENT_BACKEND", "acp"), ("TESHI_ACP_PROGRAM", sys.executable),
                    ("TESHI_ACP_ARGS_JSON", json.dumps([str(fixture), "acp", str(self.root)])),
                    ("TESHI_TEST_AGENT_CHILD", "1")]
        self.op("launch", config=dict(program=os.environ["TESHI_SELF_TEST_BIN"],
                args=[str(self.project), "--requirements-root", str(self.requirements)],
                cwd=str(self.root), env=env, cols=140, rows=36))
        self.process = True
        self.expect("Project: project-a")
        self.expect("controlled service" if self.dispatch is not None else "project-a step")

    def step(self, text, table=None):
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
